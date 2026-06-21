use std::sync::Arc;
use std::time::Duration;
use std::convert::Infallible;
use tokio::sync::{RwLock, mpsc, oneshot};
use libp2p::{PeerId, Multiaddr};
use serde::{Deserialize, Serialize};
use axum::response::sse::Event;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::P2pCommand;
use zing_cdn_core::sui::wallet::ZingWallet;
use zing_cdn_core::client::ZingClient;
use zing_cdn_core::mesh::resolver::Resolver;
use zing_cdn_core::mesh::reputation::PeerReputationTable;
use zing_cdn_core::walrus::verify::BlobVerifier;
use walrus_core::BlobId;

#[derive(Clone)]
pub struct HttpApiState {
    pub store: Arc<RwLock<BlobStore>>,
    pub pinning: Arc<RwLock<PinningManager>>,
    pub eviction: Arc<RwLock<EvictionManager>>,
    pub p2p_tx: mpsc::Sender<P2pCommand>,
    pub peer_id: PeerId,
    pub listen_addr: Multiaddr,
    pub bootstrap_peers: Arc<RwLock<Vec<String>>>,
    pub cache_dir: std::path::PathBuf,
    pub p2p_port: u16,
    pub client: Arc<ZingClient>,
    pub wallet: Option<Arc<ZingWallet>>,
}

#[derive(Deserialize)]
pub struct BlobIdQuery {
    pub blob_id: String,
}

#[derive(Serialize)]
pub struct BlobInfo {
    pub blob_id: String,
    pub size: u64,
    pub source: String,
    pub cached: bool,
    pub content: String,
    pub mime_type: String,
    pub data_base64: String,
    pub payment_error: Option<String>,
}

#[derive(Serialize)]
pub struct PeersInfo {
    pub bootstrap: Vec<String>,
    pub connected: Vec<String>,
    pub listen_addr: String,
    pub cache_dir: String,
    pub peer_id: String,
    pub p2p_addr: String,
}

#[derive(Serialize)]
pub struct HealthStatus {
    pub status: String,
    pub peer_id: String,
    pub connected_peers: usize,
}

#[derive(Serialize)]
pub struct CacheEntry {
    pub blob_id: String,
    pub size: u64,
    pub pinned: bool,
}

#[derive(Serialize)]
pub struct StakingPeerInfo {
    pub sui_address: String,
    pub peer_id_short: String,
    pub bond: u64,
    pub is_active: bool,
    pub is_live: bool,
}

#[derive(Serialize)]
pub struct MyPeerInfo {
    pub wallet_address: String,
    pub peer_id_short: Option<String>,
    pub bond: Option<u64>,
    pub is_active: Option<bool>,
    pub is_live: Option<bool>,
    pub is_registered: bool,
}

#[derive(Serialize)]
pub struct WalBalance {
    pub balance: u64,
    pub balance_wal: String,
}

#[derive(Serialize)]
pub struct RegisterResult {
    pub success: bool,
    pub message: String,
}

pub async fn handle_resolve(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
    axum::extract::Query(q): axum::extract::Query<BlobIdQuery>,
) -> Result<axum::Json<BlobInfo>, (axum::http::StatusCode, String)> {
    match resolve_blob(&state, &q.blob_id).await {
        Ok(info) => Ok(axum::Json(info)),
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

pub async fn handle_resolve_stream(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
    axum::extract::Query(q): axum::extract::Query<BlobIdQuery>,
) -> axum::response::sse::Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    use tokio_stream::wrappers::UnboundedReceiverStream;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let state_clone = state.clone();
    let blob_id = q.blob_id.clone();

    tokio::spawn(async move {
        resolve_blob_with_progress(&state_clone, &blob_id, tx).await;
    });

    axum::response::sse::Sse::new(UnboundedReceiverStream::new(rx))
}

pub async fn handle_health(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
) -> axum::Json<HealthStatus> {
    let (reply, rx) = oneshot::channel();
    let connected = match state.p2p_tx.send(P2pCommand::GetConnectedPeers { reply }).await {
        Ok(_) => rx.await.unwrap_or_default(),
        Err(_) => vec![],
    };

    axum::Json(HealthStatus {
        status: "ok".into(),
        peer_id: state.peer_id.to_string(),
        connected_peers: connected.len(),
    })
}

pub async fn handle_peers_list(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
) -> Result<axum::Json<PeersInfo>, (axum::http::StatusCode, String)> {
    match peers_list(&state).await {
        Ok(info) => Ok(axum::Json(info)),
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

pub async fn handle_peers_add(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
    axum::extract::Query(q): axum::extract::Query<PeerAddressQuery>,
) -> Result<axum::Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    match peers_add(&state, &q.addr).await {
        Ok(()) => Ok(axum::Json(serde_json::json!({"status": "ok"}))),
        Err(e) => Err((axum::http::StatusCode::BAD_REQUEST, e)),
    }
}

pub async fn handle_cache_list(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
) -> axum::Json<Vec<CacheEntry>> {
    let store = state.store.read().await;
    let pinning = state.pinning.read().await;
    let mut entries = Vec::new();
    if let Ok(ids) = store.list_blob_ids() {
        for id in ids {
            let size = store.blob_size(&id).ok().flatten().unwrap_or(0);
            let pinned = pinning.is_pinned(&id).unwrap_or(false);
            entries.push(CacheEntry { blob_id: id, size, pinned });
        }
    }
    axum::Json(entries)
}

#[derive(Deserialize)]
pub struct PeerAddressQuery {
    pub addr: String,
}

pub fn build_router(state: HttpApiState) -> axum::Router {
    let cors = tower_http::cors::CorsLayer::permissive();
    axum::Router::new()
        .route("/api/v1/resolve", axum::routing::get(handle_resolve))
        .route("/api/v1/resolve/stream", axum::routing::get(handle_resolve_stream))
        .route("/api/v1/health", axum::routing::get(handle_health))
        .route("/api/v1/peers", axum::routing::get(handle_peers_list))
        .route("/api/v1/peers/add", axum::routing::get(handle_peers_add))
        .route("/api/v1/cache", axum::routing::get(handle_cache_list))
        .route("/api/v1/staking", axum::routing::get(handle_staking))
        .route("/api/v1/my_peer", axum::routing::get(handle_my_peer))
        .route("/api/v1/balance", axum::routing::get(handle_balance))
        .route("/api/v1/register", axum::routing::post(handle_register))
        .route("/api/v1/update_peer_id", axum::routing::post(handle_update_peer_id))
        .layer(cors)
        .with_state(state)
}

async fn resolve_blob(state: &HttpApiState, blob_id: &str) -> Result<BlobInfo, String> {
    let id: BlobId = blob_id.parse().map_err(|e| format!("invalid blob id: {blob_id}: {e}"))?;

    let verifier = Arc::new(BlobVerifier::new(state.client.encoding_config_arc()));

    let mut resolver = Resolver::new(
        state.store.clone(),
        state.pinning.clone(),
        state.eviction.clone(),
        state.client.walrus_client_arc(),
        verifier,
        Arc::new(RwLock::new(PeerReputationTable::new())),
        Some(state.peer_id),
    );
    resolver.set_p2p_channel(state.p2p_tx.clone());
    if let Some(wallet) = &state.wallet {
        resolver.set_wallet(wallet.clone());
    }

    let result = resolver.resolve(&id).await.map_err(|e| e.to_string())?;
    let data = &result.data;
    let payment_error = result.payment_error.clone();

    let source = match result.resolution {
        zing_cdn_core::types::BlobResolution::LocalCache => "L0 local cache",
        zing_cdn_core::types::BlobResolution::L1Peer => "L1 peer",
        zing_cdn_core::types::BlobResolution::L3Walrus => "L3 Walrus",
    };

    let mime_type = detect_mime(data);

    let (content, data_base64) = if mime_type.starts_with("image/") {
        (
            format!("[Binary image — {} bytes]", data.len()),
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                data,
            ),
        )
    } else {
        let text = if data.len() > 2000 {
            format!("{}...", String::from_utf8_lossy(&data[..2000]))
        } else {
            String::from_utf8_lossy(data).to_string()
        };
        (text, String::new())
    };

    let _ = state.p2p_tx.send(P2pCommand::AnnounceBlob { blob_id: id.0 }).await;

    Ok(BlobInfo {
        blob_id: blob_id.to_string(),
        size: data.len() as u64,
        source: source.to_string(),
        cached: result.cached,
        content,
        mime_type: mime_type.to_string(),
        data_base64,
        payment_error,
    })
}

async fn resolve_blob_with_progress(
    state: &HttpApiState,
    blob_id: &str,
    tx: tokio::sync::mpsc::UnboundedSender<Result<Event, Infallible>>,
) {
    let ev = |v: serde_json::Value| Ok(Event::default().data(v.to_string()));
    let send = |v: serde_json::Value| { let _ = tx.send(ev(v)); };
    let send_err = |msg: &str| {
        send(serde_json::json!({"type":"error","error":msg}));
    };

    let id: BlobId = match blob_id.parse() {
        Ok(id) => id,
        Err(e) => { send_err(&format!("invalid blob id: {e}")); return; }
    };

    send(serde_json::json!({"type":"status","status":"Checking local cache...","layer":"L0"}));

    {
        let store = state.store.read().await;
        if let Ok(Some(data)) = store.get(blob_id) {
            let info = build_blobinfo(blob_id, &data, "L0 local cache", true, None);
            send(serde_json::json!({"type":"result","info":{
                "blob_id": info.blob_id, "size": info.size, "source": info.source,
                "cached": info.cached, "content": info.content, "mime_type": info.mime_type,
                "data_base64": info.data_base64, "payment_error": info.payment_error
            }}));
            let _ = state.p2p_tx.send(P2pCommand::AnnounceBlob { blob_id: id.0 }).await;
            return;
        }
    }

    send(serde_json::json!({"type":"status","status":"Searching P2P network...","layer":"L1"}));

    let verifier = Arc::new(BlobVerifier::new(state.client.encoding_config_arc()));
    let mut resolver = Resolver::new(
        state.store.clone(),
        state.pinning.clone(),
        state.eviction.clone(),
        state.client.walrus_client_arc(),
        verifier,
        Arc::new(RwLock::new(PeerReputationTable::new())),
        Some(state.peer_id),
    );
    resolver.set_p2p_channel(state.p2p_tx.clone());
    if let Some(wallet) = &state.wallet {
        resolver.set_wallet(wallet.clone());
    }

    match resolver.resolve(&id).await {
        Ok(result) => {
            let source = match result.resolution {
                zing_cdn_core::types::BlobResolution::LocalCache => "L0 local cache",
                zing_cdn_core::types::BlobResolution::L1Peer => "L1 peer",
                zing_cdn_core::types::BlobResolution::L3Walrus => "L3 Walrus",
            };
            send(serde_json::json!({"type":"status","status":format!("Resolved via {source}"),"layer":&source[..2],"source":source}));
            let info = build_blobinfo(blob_id, &result.data, source, result.cached, result.payment_error.clone());
            let _ = state.p2p_tx.send(P2pCommand::AnnounceBlob { blob_id: id.0 }).await;
            send(serde_json::json!({"type":"result","info":{
                "blob_id": info.blob_id, "size": info.size, "source": info.source,
                "cached": info.cached, "content": info.content, "mime_type": info.mime_type,
                "data_base64": info.data_base64, "payment_error": info.payment_error
            }}));
        }
        Err(e) => send_err(&e.to_string()),
    }
}

fn build_blobinfo(blob_id: &str, data: &[u8], source: &str, cached: bool, payment_error: Option<String>) -> BlobInfo {
    let mime_type = detect_mime(data);
    let (content, data_base64) = if mime_type.starts_with("image/") {
        (
            format!("[Binary image — {} bytes]", data.len()),
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data),
        )
    } else {
        let text = if data.len() > 2000 {
            format!("{}...", String::from_utf8_lossy(&data[..2000]))
        } else {
            String::from_utf8_lossy(data).to_string()
        };
        (text, String::new())
    };
    BlobInfo {
        blob_id: blob_id.to_string(),
        size: data.len() as u64,
        source: source.to_string(),
        cached,
        content,
        mime_type: mime_type.to_string(),
        data_base64,
        payment_error,
    }
}

fn detect_mime(data: &[u8]) -> &'static str {
    if (data.starts_with(b"{") || data.starts_with(b"[") || data.starts_with(b"<"))
        && String::from_utf8_lossy(data).chars().all(|c| c.is_ascii_graphic() || c.is_ascii_whitespace()) {
        return "application/json";
    }
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) { "image/png" }
    else if data.starts_with(&[0xFF, 0xD8, 0xFF]) { "image/jpeg" }
    else if data.starts_with(&[0x47, 0x49, 0x46, 0x38]) { "image/gif" }
    else if data.len() > 8 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" { "image/webp" }
    else { "text/plain" }
}

pub async fn peers_list(state: &HttpApiState) -> Result<PeersInfo, String> {
    let (reply, rx) = oneshot::channel();
    state.p2p_tx.send(P2pCommand::GetConnectedPeers { reply }).await.map_err(|e| e.to_string())?;
    let connected = rx.await.map_err(|e| e.to_string())?;

    let bootstrap = state.bootstrap_peers.read().await.clone();
    let p2p_addr = format!("/ip4/127.0.0.1/udp/{}/quic-v1/p2p/{}", state.p2p_port, state.peer_id);

    Ok(PeersInfo {
        bootstrap,
        connected: connected.iter().map(|p| p.to_string()).collect(),
        listen_addr: state.listen_addr.to_string(),
        cache_dir: state.cache_dir.display().to_string(),
        peer_id: state.peer_id.to_string(),
        p2p_addr,
    })
}

pub async fn peers_add(state: &HttpApiState, addr_str: &str) -> Result<(), String> {
    use libp2p::multiaddr::Protocol;

    let addr: Multiaddr = addr_str.parse().map_err(|_| {
        "invalid multiaddr — expected format: /ip4/<ip>/udp/<port>/quic-v1/p2p/<peer_id>".to_string()
    })?;
    let mut peer_id = None;
    for proto in addr.iter() {
        if let Protocol::P2p(peer) = proto {
            peer_id = Some(peer);
            break;
        }
    }
    let peer_id = peer_id.ok_or("multiaddr must contain /p2p/ protocol")?;

    let mut addr_no_p2p = addr.clone();
    addr_no_p2p.pop();

    state.p2p_tx.send(P2pCommand::AddBootstrapPeer { peer_id, addr: addr_no_p2p.clone() }).await.map_err(|e| e.to_string())?;
    state.p2p_tx.send(P2pCommand::Dial { peer_id, addr: addr_no_p2p }).await.map_err(|e| e.to_string())?;

    for _ in 0..15 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (reply, rx) = oneshot::channel();
        if state.p2p_tx.send(P2pCommand::GetConnectedPeers { reply }).await.is_err() {
            break;
        }
        if let Ok(connected) = rx.await {
            if connected.contains(&peer_id) {
                break;
            }
        }
    }

    let _ = state.p2p_tx.send(P2pCommand::Bootstrap).await;

    let mut peers = state.bootstrap_peers.write().await;
    if !peers.contains(&addr_str.to_string()) {
        peers.push(addr_str.to_string());
    }

    Ok(())
}

pub async fn handle_staking(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
) -> Result<axum::Json<Vec<StakingPeerInfo>>, (axum::http::StatusCode, String)> {
    let wallet = state.wallet.as_ref()
        .ok_or_else(|| (axum::http::StatusCode::BAD_REQUEST, "Wallet not configured".into()))?;

    let peers = wallet.list_all_peers().await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (reply, rx) = oneshot::channel();
    state.p2p_tx.send(P2pCommand::GetConnectedPeers { reply }).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let connected = rx.await.map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let connected_ids: std::collections::HashSet<String> =
        connected.iter().map(|p| p.to_string()).collect();

    let result = peers.into_iter().map(|p| {
        let is_live = connected_ids.contains(&p.peer_id_b58);
        let short = if p.peer_id_b58.len() > 16 {
            format!("{}...{}", &p.peer_id_b58[..8], &p.peer_id_b58[p.peer_id_b58.len() - 8..])
        } else {
            p.peer_id_b58.clone()
        };
        StakingPeerInfo {
            sui_address: p.sui_address,
            peer_id_short: short,
            bond: p.bond,
            is_active: p.is_active,
            is_live,
        }
    }).collect();

    Ok(axum::Json(result))
}

pub async fn handle_my_peer(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
) -> Result<axum::Json<MyPeerInfo>, (axum::http::StatusCode, String)> {
    let wallet = state.wallet.as_ref()
        .ok_or_else(|| (axum::http::StatusCode::BAD_REQUEST, "Wallet not configured".into()))?;

    let wallet_address = format!("{:#}", wallet.address());

    let peers = wallet.list_all_peers().await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (reply, rx) = oneshot::channel();
    state.p2p_tx.send(P2pCommand::GetConnectedPeers { reply }).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let connected = rx.await.map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let connected_ids: std::collections::HashSet<String> =
        connected.iter().map(|p| p.to_string()).collect();

    let my_peer = peers.into_iter().find(|p| {
        p.sui_address == wallet_address
    });

    let info = match my_peer {
        Some(p) => {
            let is_live = connected_ids.contains(&p.peer_id_b58);
            let short = if p.peer_id_b58.len() > 16 {
                format!("{}...{}", &p.peer_id_b58[..8], &p.peer_id_b58[p.peer_id_b58.len() - 8..])
            } else {
                p.peer_id_b58.clone()
            };
            MyPeerInfo {
                wallet_address,
                peer_id_short: Some(short),
                bond: Some(p.bond),
                is_active: Some(p.is_active),
                is_live: Some(is_live),
                is_registered: true,
            }
        }
        None => MyPeerInfo {
            wallet_address,
            peer_id_short: None,
            bond: None,
            is_active: None,
            is_live: None,
            is_registered: false,
        },
    };

    Ok(axum::Json(info))
}

pub async fn handle_balance(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
) -> Result<axum::Json<WalBalance>, (axum::http::StatusCode, String)> {
    let wallet = state.wallet.as_ref()
        .ok_or_else(|| (axum::http::StatusCode::BAD_REQUEST, "Wallet not configured".into()))?;

    let balance = wallet.get_wal_balance().await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let balance_wal = format!("{}.{:09}", balance / 1_000_000_000, balance % 1_000_000_000);

    Ok(axum::Json(WalBalance { balance, balance_wal }))
}

pub async fn handle_register(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
) -> Result<axum::Json<RegisterResult>, (axum::http::StatusCode, String)> {
    let wallet = state.wallet.as_ref()
        .ok_or_else(|| (axum::http::StatusCode::BAD_REQUEST, "Wallet not configured".into()))?;

    let peer_id_bytes = state.peer_id.to_bytes();

    wallet.register_peer(peer_id_bytes).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(axum::Json(RegisterResult {
        success: true,
        message: "Peer registered successfully".into(),
    }))
}

pub async fn handle_update_peer_id(
    axum::extract::State(state): axum::extract::State<HttpApiState>,
) -> Result<axum::Json<RegisterResult>, (axum::http::StatusCode, String)> {
    let wallet = state.wallet.as_ref()
        .ok_or_else(|| (axum::http::StatusCode::BAD_REQUEST, "Wallet not configured".into()))?;

    let peer_id_bytes = state.peer_id.to_bytes();

    wallet.update_peer_id(peer_id_bytes).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(axum::Json(RegisterResult {
        success: true,
        message: "Peer ID updated successfully".into(),
    }))
}
