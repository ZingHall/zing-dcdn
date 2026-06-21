#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_http;

use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tauri::Manager;
use libp2p::Multiaddr;
use tracing_subscriber::EnvFilter;
use axum::{routing, Json, extract::{State, Query}, response::sse::{Event, Sse}};
use serde::Deserialize;
use tokio_stream::wrappers::UnboundedReceiverStream;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::node::ZingP2pNode;
use zing_cdn_core::sui::wallet::ZingWallet;
use zing_cdn_core::sui::settlement::SettlementConfig;
use zing_cdn_core::config::ZingConfig;
use zing_cdn_core::client::ZingClient;

use crate::api_http::HttpApiState;

const CACHE_BUDGET: u64 = 500 * 1024 * 1024;

const DEFAULT_BOOTSTRAP: &[&str] = &[];

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

            let keystore_path: Option<std::path::PathBuf> = std::env::var("ZING_SUI_KEYSTORE")
                .ok()
                .map(std::path::PathBuf::from);

            // Load settlement config from ~/.zing-cdn/config.toml
            let config = ZingConfig::load();
            let settlement_cfg = config.settlement.as_ref();
            let settlement_config: Option<SettlementConfig> = settlement_cfg.and_then(|c| {
                let package_id = c.package.as_ref()?.parse().ok()?;
                let settlement_object_id = c.settlement_object.as_ref()?.parse().ok()?;
                let vault_object_id = c.vault_object.as_ref().and_then(|v| v.parse().ok());
                let peer_vaults_table_str = c.peer_vaults_table.as_deref()
                    .unwrap_or("0x465bf3e99dff79a56705b111396ee5b9bd35f2a1aac70d118f466a7c581e0e07");
                let peer_vaults_table_stripped = peer_vaults_table_str.strip_prefix("0x").unwrap_or(peer_vaults_table_str);
                let mut peer_vaults_table_id = [0u8; 32];
                if peer_vaults_table_stripped.len() == 64 {
                    for i in 0..32 {
                        peer_vaults_table_id[i] = u8::from_str_radix(&peer_vaults_table_stripped[i*2..i*2+2], 16).unwrap_or(0);
                    }
                }
                let peer_vault_registry_id = c.peer_vault_registry.as_ref()
                    .and_then(|v| v.parse().ok());
                Some(SettlementConfig {
                    package_id,
                    settlement_object_id,
                    registry_object_id: c.registry_object.as_ref()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(|| "0x97b5153b9e9897ad1630cdd06e5caa81ebbf8865e96003f38e50c5f1d6752527".parse().unwrap()),
                    registry_peers_table_id: c.registry_peers_table.as_ref()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(|| "0xbcd17d4df8489569fdca7bc9a795c16a73560efbde2355d91ef9195bf676ea00".parse().unwrap()),
                    peer_vaults_table_id,
                    peer_vault_registry_id,
                    vault_object_id,
                    wal_coin_type: "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL".into(),
                    wal_package_id: "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59".parse().unwrap(),
                    registry_initial_shared_version: c.registry_version.unwrap_or(921074118),
                    settlement_initial_shared_version: c.settlement_version.unwrap_or(921074118),
                    vault_initial_shared_version: c.vault_version.unwrap_or(921074119),
                    peer_vaults_initial_shared_version: c.peer_vaults_version.unwrap_or(923306507),
                    peer_vault_registry_initial_shared_version: c.peer_vault_registry_version.unwrap_or(923306507),
                    share_certificate_type: "0x9dd1a5dc551e322dd1b0394514ece30eb1e5f54d5de5b1f6fe135ebe24032b9c::peer_vault::ShareCertificate".into(),
                })
            });

            let wallet = Arc::new(
                tauri::async_runtime::block_on(ZingWallet::from_keystore(keystore_path.as_deref(), settlement_config.clone()))
                    .expect("Sui wallet required for GUI"),
            );
            tracing::info!(address = %wallet.address(), "Sui wallet loaded for WAL payments");
            let sui_address_bytes: Option<[u8; 32]> = {
                let addr = wallet.address();
                let bytes: [u8; 32] = addr.into();
                Some(bytes)
            };

            let keypair = wallet.to_libp2p_keypair();
            let (p2p_node, command_rx) = ZingP2pNode::new(store.clone(), keypair);
            let p2p_tx = p2p_node.command_tx().clone();
            let p2p_key = p2p_node.key().clone();
            let peer_id = p2p_node.local_peer_id();
            tracing::info!(%peer_id, "P2P keypair derived from wallet");

            // Parse vault object ID for Kad DHT publishing
            let vault_object_id_bytes: Option<[u8; 32]> = settlement_cfg
                .and_then(|c| c.vault_object.as_ref())
                .and_then(|v| {
                    let hex_str = v.strip_prefix("0x").unwrap_or(v);
                    if hex_str.len() != 64 { return None; }
                    let mut bytes = [0u8; 32];
                    for i in 0..32 {
                        bytes[i] = u8::from_str_radix(&hex_str[i*2..i*2+2], 16).ok()?;
                    }
                    Some(bytes)
                });

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
                wallet: Some(wallet.clone()),
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
                .route("/api/staking", routing::get(handle_staking))
                .route("/api/my_peer", routing::get(handle_my_peer))
                .route("/api/balance", routing::get(handle_balance))
                .route("/api/register", routing::post(handle_register))
                .route("/api/update_peer_id", routing::post(handle_update_peer_id))
                .route("/api/my_vault", routing::get(handle_my_vault))
                .route("/api/create_vault", routing::post(handle_create_vault))
                .route("/api/my_shares", routing::get(handle_my_shares))
                .route("/api/claim_earnings", routing::post(handle_claim_earnings))
                .route("/api/delegate", routing::post(handle_delegate))
                .route("/api/undelegate", routing::post(handle_undelegate))
                .layer(cors)
                .with_state(api_state);

            tracing::info!("HTTP API binding to 127.0.0.1:{api_port}");

            // Start axum in tokio
            let bind_addr = format!("127.0.0.1:{api_port}");
            tauri::async_runtime::spawn(async move {
                let listener = tokio::net::TcpListener::bind(&bind_addr)
                    .await
                    .unwrap_or_else(|_| panic!("bind http api on {bind_addr}"));
                tracing::info!("HTTP API listening on {bind_addr}");
                axum::serve(listener, app_router).await.ok();
            });

            // Spawn P2P background task with loaded bootstrap peers
            tauri::async_runtime::spawn(async move {
                let _ = ZingP2pNode::run(
                    p2p_key, command_rx, store, listen_addr, bootstrap_addrs, external_addrs, sui_address_bytes, vault_object_id_bytes,
                ).await;
            });

            let window = app.get_webview_window("main").unwrap();
            window.eval(format!("window.ZING_API_PORT = {api_port};")).ok();
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

async fn handle_staking(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::list_staking(&state).await {
        Ok(peers) => Json(serde_json::to_value(peers).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_my_peer(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::get_my_peer_info(&state).await {
        Ok(info) => Json(serde_json::to_value(info).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_balance(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::get_wal_balance(&state).await {
        Ok(info) => Json(serde_json::to_value(info).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_register(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::register_peer(&state).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_update_peer_id(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    let wallet = match state.wallet.as_ref() {
        Some(w) => w,
        None => return Json(serde_json::json!({"error": "Wallet not configured"})),
    };
    let peer_id_bytes = state.peer_id.to_bytes();
    match wallet.update_peer_id(peer_id_bytes).await {
        Ok(()) => Json(serde_json::json!({"success": true, "message": "Peer ID updated successfully"})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

async fn handle_my_vault(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::get_my_vault(&state).await {
        Ok(info) => Json(serde_json::to_value(info).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_create_vault(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::create_vault(&state).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_my_shares(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::list_my_shares(&state).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_claim_earnings(State(state): State<HttpApiState>) -> Json<serde_json::Value> {
    match api_http::claim_earnings(&state).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_delegate(State(state): State<HttpApiState>, axum::extract::Query(q): axum::extract::Query<api_http::DelegateQuery>) -> Json<serde_json::Value> {
    match api_http::delegate(&state, &q.vault_object_id, &q.amount).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn handle_undelegate(State(state): State<HttpApiState>, axum::extract::Query(q): axum::extract::Query<api_http::CertObjectIdQuery>) -> Json<serde_json::Value> {
    match api_http::undelegate(&state, &q.cert_object_id).await {
        Ok(result) => Json(serde_json::to_value(result).unwrap()),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}
