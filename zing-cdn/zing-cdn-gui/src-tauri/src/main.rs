#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_http;

use std::sync::Arc;
use tokio::sync::RwLock;
use libp2p::Multiaddr;
use axum::{routing, Json, extract::{State, Query}};
use serde::Deserialize;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::node::ZingP2pNode;

use crate::api_http::HttpApiState;

const CACHE_BUDGET: u64 = 500 * 1024 * 1024;

fn main() {
    tauri::Builder::default()
        .setup(|_app| {
            let cache_dir = dirs::home_dir()
                .unwrap_or_default()
                .join(".zing-cdn")
                .join("cache");
            std::fs::create_dir_all(&cache_dir).expect("create cache dir");

            let store = Arc::new(RwLock::new(
                BlobStore::open(&cache_dir).expect("open blob store"),
            ));

            let (p2p_node, command_rx) = ZingP2pNode::new(store.clone());
            let p2p_tx = p2p_node.command_tx().clone();
            let p2p_key = p2p_node.key().clone();
            let peer_id = p2p_node.local_peer_id();
            let listen_addr: Multiaddr = "/ip4/0.0.0.0/udp/34291/quic-v1"
                .parse()
                .expect("valid listen addr");

            let pinning = Arc::new(RwLock::new(PinningManager::new(
                store.blocking_read().clone(),
            )));
            let eviction = Arc::new(RwLock::new(EvictionManager::new(
                store.blocking_read().clone(),
                CACHE_BUDGET,
            )));

            let api_state = HttpApiState {
                store: store.clone(),
                pinning: Arc::clone(&pinning),
                eviction,
                p2p_tx: p2p_tx.clone(),
                peer_id,
                listen_addr: listen_addr.clone(),
            };

            // Build axum router with CORS (localhost app — permissive)
            let cors = tower_http::cors::CorsLayer::permissive();
            let app_router = axum::Router::new()
                .route("/api/dashboard", routing::get(handle_dashboard))
                .route("/api/cache", routing::get(handle_list_cache))
                .route("/api/resolve_blob", routing::get(handle_resolve))
                .route("/api/pin", routing::get(handle_pin))
                .route("/api/unpin", routing::get(handle_unpin))
                .route("/api/delete", routing::get(handle_delete))
                .layer(cors)
                .with_state(api_state);

            eprintln!("HTTP API binding to 127.0.0.1:13420");

            // Start axum in tokio
            tauri::async_runtime::spawn(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:13420")
                    .await
                    .expect("bind http api on 127.0.0.1:13420");
                eprintln!("HTTP API listening on 127.0.0.1:13420");
                axum::serve(listener, app_router).await.ok();
            });

            // Spawn P2P background task
            tauri::async_runtime::spawn(async move {
                let _ = ZingP2pNode::run(
                    p2p_key, command_rx, store, listen_addr, vec![],
                ).await;
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[derive(Deserialize)]
struct BlobIdQuery {
    blob_id: String,
}

async fn handle_dashboard(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::get_dashboard(&state).await {
        Ok(info) => Json(serde_json::to_value(info).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_list_cache(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::list_cache(&state).await {
        Ok(entries) => Json(serde_json::to_value(entries).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_resolve(State(state): State<HttpApiState>, Query(q): Query<BlobIdQuery>) -> Json<serde_json::Value> {
    match api_http::resolve_blob(&state, &q.blob_id).await {
        Ok(info) => Json(serde_json::to_value(info).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_pin(State(state): State<HttpApiState>, Query(q): Query<BlobIdQuery>) -> Json<serde_json::Value> {
    match api_http::pin_blob(&state, &q.blob_id).await {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_unpin(State(state): State<HttpApiState>, Query(q): Query<BlobIdQuery>) -> Json<serde_json::Value> {
    match api_http::unpin_blob(&state, &q.blob_id).await {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_delete(State(state): State<HttpApiState>, Query(q): Query<BlobIdQuery>) -> Json<serde_json::Value> {
    match api_http::delete_blob(&state, &q.blob_id).await {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}
