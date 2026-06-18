#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_http;

use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tauri::Manager;
use libp2p::{Multiaddr, identity};
use tracing_subscriber::EnvFilter;
use axum::{routing, Json, extract::{State, Query}, response::sse::{Event, Sse}};
use serde::Deserialize;
use tokio_stream::wrappers::UnboundedReceiverStream;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::node::ZingP2pNode;
use zing_cdn_core::client::ZingClient;

use crate::api_http::HttpApiState;

const CACHE_BUDGET: u64 = 500 * 1024 * 1024;

const DEFAULT_BOOTSTRAP: &[&str] = &[];

fn load_or_generate_keypair(cache_dir: &Path) -> identity::Keypair {
    let path = cache_dir.join("keypair");
    if let Ok(data) = std::fs::read(&path) {
        if let Ok(kp) = identity::Keypair::from_protobuf_encoding(&data) {
            return kp;
        }
    }
    let kp = identity::Keypair::generate_ed25519();
    let data = kp.to_protobuf_encoding().expect("serialize keypair");
    if std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, &data))
        .is_err()
    {
        if let Ok(data) = std::fs::read(&path) {
            if let Ok(kp) = identity::Keypair::from_protobuf_encoding(&data) {
                return kp;
            }
        }
    }
    kp
}

fn load_peers(cache_dir: &Path) -> Vec<String> {
    let path = cache_dir.join("peers.json");
    if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        vec![]
    }
}

fn save_peers(peers: &[String], cache_dir: &Path) {
    let path = cache_dir.join("peers.json");
    if let Ok(json) = serde_json::to_string(peers) {
        std::fs::write(path, json).ok();
    }
}

fn parse_bootstrap_peers(peers: &[String]) -> Vec<(libp2p::PeerId, Multiaddr)> {
    use libp2p::multiaddr::Protocol;
    peers.iter().filter_map(|s| {
        let mut addr: Multiaddr = s.parse().ok()?;
        let peer_id = addr.iter().find_map(|p| {
            if let Protocol::P2p(peer) = p {
                Some(peer)
            } else {
                None
            }
        })?;
        addr.pop();
        Some((peer_id, addr))
    }).collect()
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let cache_dir = std::env::var("ZING_CACHE_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    dirs::home_dir()
                        .unwrap_or_default()
                        .join(".zing-cdn")
                        .join("cache")
                });
            std::fs::create_dir_all(&cache_dir).expect("create cache dir");

            let log_dir = cache_dir.clone();
            let file_appender = tracing_appender::rolling::never(&log_dir, "zing-cdn.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new("info"))
                )
                .with_writer(non_blocking)
                .with_ansi(false)
                .init();
            std::mem::forget(guard);

            let p2p_port: u16 = std::env::var("ZING_P2P_PORT")
                .unwrap_or_else(|_| "34291".into())
                .parse()
                .unwrap_or(34291);
            let api_port: u16 = std::env::var("ZING_API_PORT")
                .unwrap_or_else(|_| "13420".into())
                .parse()
                .unwrap_or(13420);
            let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/udp/{p2p_port}/quic-v1")
                .parse()
                .expect("valid listen addr");

            let external_addrs: Vec<Multiaddr> = std::env::var("ZING_EXTERNAL_ADDR")
                .map(|s| {
                    s.split(',')
                        .map(|a| a.trim().to_string())
                        .filter(|a| !a.is_empty())
                        .map(|a| a.parse().expect("valid ZING_EXTERNAL_ADDR"))
                        .collect()
                })
                .unwrap_or_default();

            tracing::info!("P2P port: {p2p_port}, API port: {api_port}, cache: {}", cache_dir.display());

            let store = Arc::new(RwLock::new(
                BlobStore::open(&cache_dir).expect("open blob store"),
            ));

            let keypair = load_or_generate_keypair(&cache_dir);
            let (p2p_node, command_rx) = ZingP2pNode::new(store.clone(), keypair);
            let p2p_tx = p2p_node.command_tx().clone();
            let p2p_key = p2p_node.key().clone();
            let peer_id = p2p_node.local_peer_id();

            tracing::info!("PeerId: {peer_id}");

            let client = Arc::new(
                tauri::async_runtime::block_on(ZingClient::from_mainnet())
                    .expect("connect to Walrus mainnet"),
            );

            let pinning = Arc::new(RwLock::new(PinningManager::new(
                store.blocking_read().clone(),
            )));
            let eviction = Arc::new(RwLock::new(EvictionManager::new(
                store.blocking_read().clone(),
                CACHE_BUDGET,
            )));

            let mut peers_str = load_peers(&cache_dir);
            for bp in DEFAULT_BOOTSTRAP {
                let bp_str = bp.to_string();
                if !peers_str.contains(&bp_str) {
                    peers_str.push(bp_str);
                }
            }
            let bootstrap_addrs = parse_bootstrap_peers(&peers_str);
            let bootstrap_peers = Arc::new(RwLock::new(peers_str));

            let api_state = HttpApiState {
                store: store.clone(),
                pinning: Arc::clone(&pinning),
                eviction,
                p2p_tx: p2p_tx.clone(),
                peer_id,
                listen_addr: listen_addr.clone(),
                bootstrap_peers: bootstrap_peers.clone(),
                cache_dir: cache_dir.clone(),
                p2p_port,
                client,
            };

            // Build axum router with CORS (localhost app — permissive)
            let cors = tower_http::cors::CorsLayer::permissive();
            let app_router = axum::Router::new()
                .route("/api/dashboard", routing::get(handle_dashboard))
                .route("/api/cache", routing::get(handle_list_cache))
                .route("/api/resolve_blob", routing::get(handle_resolve))
                .route("/api/resolve_blob_stream", routing::get(handle_resolve_stream))
                .route("/api/pin", routing::get(handle_pin))
                .route("/api/unpin", routing::get(handle_unpin))
                .route("/api/delete", routing::get(handle_delete))
                .route("/api/peers", routing::get(handle_peers_list))
                .route("/api/peers/add", routing::post(handle_peers_add))
                .route("/api/peers/remove", routing::post(handle_peers_remove))
                .layer(cors)
                .with_state(api_state);

            tracing::info!("HTTP API binding to 127.0.0.1:{api_port}");

            // Start axum in tokio
            let bind_addr = format!("127.0.0.1:{api_port}");
            tauri::async_runtime::spawn(async move {
                let listener = tokio::net::TcpListener::bind(&bind_addr)
                    .await
                    .expect(&format!("bind http api on {bind_addr}"));
                tracing::info!("HTTP API listening on {bind_addr}");
                axum::serve(listener, app_router).await.ok();
            });

            // Spawn P2P background task with loaded bootstrap peers
            tauri::async_runtime::spawn(async move {
                let _ = ZingP2pNode::run(
                    p2p_key, command_rx, store, listen_addr, bootstrap_addrs, external_addrs, None,
                ).await;
            });

            let window = app.get_webview_window("main").unwrap();
            window.eval(&format!("window.ZING_API_PORT = {api_port};")).ok();
            window.set_title(&format!("zing-cdn :{api_port}")).ok();

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

async fn handle_resolve_stream(
    State(state): State<HttpApiState>,
    Query(q): Query<BlobIdQuery>,
) -> Sse<UnboundedReceiverStream<Result<Event, std::convert::Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let state_clone = state.clone();
    let blob_id = q.blob_id.clone();

    tokio::spawn(async move {
        api_http::resolve_blob_with_progress(&state_clone, &blob_id, tx).await;
    });

    Sse::new(UnboundedReceiverStream::new(rx))
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

#[derive(Deserialize)]
struct AddrRequest {
    addr: String,
}

async fn handle_peers_list(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::peers_list(&state).await {
        Ok(info) => Json(serde_json::to_value(info).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_peers_add(
    State(state): State<HttpApiState>,
    Json(body): Json<AddrRequest>,
) -> Json<serde_json::Value> {
    match api_http::peers_add(&state, &body.addr).await {
        Ok(()) => {
            let peers = state.bootstrap_peers.read().await.clone();
            save_peers(&peers, &state.cache_dir);
            Json(serde_json::json!({"ok": true}))
        }
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_peers_remove(
    State(state): State<HttpApiState>,
    Json(body): Json<AddrRequest>,
) -> Json<serde_json::Value> {
    match api_http::peers_remove(&state, &body.addr).await {
        Ok(()) => {
            let peers = state.bootstrap_peers.read().await.clone();
            save_peers(&peers, &state.cache_dir);
            Json(serde_json::json!({"ok": true}))
        }
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}
