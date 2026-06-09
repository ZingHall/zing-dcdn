use std::sync::Arc;
use std::time::Duration;
use std::convert::Infallible;
use tokio::sync::{RwLock, mpsc, oneshot};
use libp2p::{PeerId, Multiaddr};
use serde::Serialize;
use axum::response::sse::Event;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::P2pCommand;
use zing_cdn_core::client::ZingClient;

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
    #[allow(dead_code)]
    pub api_port: u16,
    pub client: Arc<ZingClient>,
}

const CACHE_BUDGET: u64 = 500 * 1024 * 1024;

#[derive(Serialize)]
pub struct DashboardInfo {
    pub peer_id: String,
    pub listen_addr: String,
    pub connected_peers: Vec<String>,
    pub cache_used: u64,
    pub cache_budget: u64,
    pub cache_count: usize,
}

#[derive(Serialize)]
pub struct CacheEntry {
    pub blob_id: String,
    pub size: u64,
    pub pinned: bool,
}

pub async fn get_dashboard(state: &HttpApiState) -> Result<DashboardInfo, String> {
    let store = state.store.read().await;
    let ids = store.list_blob_ids().map_err(|e| e.to_string())?;
    let cache_count = ids.len();
    let cache_used = store.total_size().map_err(|e| e.to_string())?;

    let (reply, rx) = oneshot::channel();
    state.p2p_tx.send(P2pCommand::GetConnectedPeers { reply }).await.map_err(|e| e.to_string())?;
    let connected = rx.await.map_err(|e| e.to_string())?;

    Ok(DashboardInfo {
        peer_id: state.peer_id.to_string(),
        listen_addr: state.listen_addr.to_string(),
        connected_peers: connected.iter().map(|p| p.to_string()).collect(),
        cache_used,
        cache_budget: CACHE_BUDGET,
        cache_count,
    })
}

pub async fn list_cache(state: &HttpApiState) -> Result<Vec<CacheEntry>, String> {
    let store = state.store.read().await;
    let pinning = state.pinning.read().await;
    let mut entries = Vec::new();
    for id in store.list_blob_ids().map_err(|e| e.to_string())? {
        let size = store.blob_size(&id).map_err(|e| e.to_string())?.unwrap_or(0);
        let pinned = pinning.is_pinned(&id).map_err(|e| e.to_string())?;
        entries.push(CacheEntry { blob_id: id, size, pinned });
    }
    Ok(entries)
}

pub async fn pin_blob(state: &HttpApiState, blob_id: &str) -> Result<(), String> {
    state.pinning.read().await.pin(blob_id).map_err(|e| e.to_string())
}

pub async fn unpin_blob(state: &HttpApiState, blob_id: &str) -> Result<(), String> {
    state.pinning.read().await.unpin(blob_id).map_err(|e| e.to_string())
}

pub async fn delete_blob(state: &HttpApiState, blob_id: &str) -> Result<(), String> {
    state.store.write().await.delete(blob_id).map_err(|e| e.to_string())
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
}

fn detect_mime(data: &[u8]) -> &'static str {
    if data.len() < 4 {
        return "application/octet-stream";
    }
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) { "image/png" }
    else if data.starts_with(&[0xFF, 0xD8, 0xFF]) { "image/jpeg" }
    else if data.starts_with(&[0x47, 0x49, 0x46, 0x38]) { "image/gif" }
    else if data.len() > 8 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" { "image/webp" }
    else { "text/plain" }
}

pub async fn resolve_blob(state: &HttpApiState, blob_id: &str) -> Result<BlobInfo, String> {
    use zing_cdn_core::mesh::resolver::Resolver;
    use zing_cdn_core::mesh::reputation::PeerReputationTable;
    use zing_cdn_core::walrus::verify::BlobVerifier;
    use walrus_core::BlobId;

    let id: BlobId = blob_id.parse().map_err(|e| format!("invalid blob id: {blob_id}: {e}"))?;

    let verifier = Arc::new(BlobVerifier::new(state.client.encoding_config_arc()));

    let mut resolver = Resolver::new(
        state.store.clone(),
        state.pinning.clone(),
        state.eviction.clone(),
        state.client.walrus_client_arc(),
        verifier,
        Arc::new(RwLock::new(PeerReputationTable::new())),
    );
    resolver.set_p2p_channel(state.p2p_tx.clone());

    let result = resolver.resolve(&id).await.map_err(|e| e.to_string())?;
    let data = &result.data;

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

    // Announce blob via P2P DHT so peers can discover it
    let _ = state.p2p_tx.send(P2pCommand::AnnounceBlob { blob_id: id.0 }).await;

    Ok(BlobInfo {
        blob_id: blob_id.to_string(),
        size: data.len() as u64,
        source: source.to_string(),
        cached: result.cached,
        content,
        mime_type: mime_type.to_string(),
        data_base64,
    })
}

fn build_blobinfo(blob_id: &str, data: &[u8], source: &str, cached: bool) -> BlobInfo {
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
    }
}

pub async fn resolve_blob_with_progress(
    state: &HttpApiState,
    blob_id: &str,
    tx: tokio::sync::mpsc::UnboundedSender<Result<Event, Infallible>>,
) {
    use zing_cdn_core::mesh::resolver::Resolver;
    use zing_cdn_core::mesh::reputation::PeerReputationTable;
    use zing_cdn_core::walrus::verify::BlobVerifier;
    use walrus_core::BlobId;

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
            let info = build_blobinfo(blob_id, &data, "L0 local cache", true);
            send(serde_json::json!({"type":"result","info":{
                "blob_id": info.blob_id, "size": info.size, "source": info.source,
                "cached": info.cached, "content": info.content, "mime_type": info.mime_type,
                "data_base64": info.data_base64
            }}));
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
    );
    resolver.set_p2p_channel(state.p2p_tx.clone());

    match resolver.resolve(&id).await {
        Ok(result) => {
            let source = match result.resolution {
                zing_cdn_core::types::BlobResolution::LocalCache => "L0 local cache",
                zing_cdn_core::types::BlobResolution::L1Peer => "L1 peer",
                zing_cdn_core::types::BlobResolution::L3Walrus => "L3 Walrus",
            };
            send(serde_json::json!({"type":"status","status":format!("Resolved via {source}"),"layer":&source[..2],"source":source}));
            let info = build_blobinfo(blob_id, &result.data, source, result.cached);
            let _ = state.p2p_tx.send(P2pCommand::AnnounceBlob { blob_id: id.0 }).await;
            send(serde_json::json!({"type":"result","info":{
                "blob_id": info.blob_id, "size": info.size, "source": info.source,
                "cached": info.cached, "content": info.content, "mime_type": info.mime_type,
                "data_base64": info.data_base64
            }}));
        }
        Err(e) => send_err(&e.to_string()),
    }
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

    let addr: Multiaddr = addr_str.parse().map_err(|e| format!("invalid multiaddr: {e}"))?;
    let mut peer_id = None;
    for proto in addr.iter() {
        if let Protocol::P2p(peer) = proto {
            peer_id = Some(peer);
            break;
        }
    }
    let peer_id = peer_id.ok_or("multiaddr must contain /p2p/ protocol")?;

    // Strip /p2p/ suffix — the Dial handler in node.rs adds it back
    // to avoid double /p2p/ stacking
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

pub async fn peers_remove(state: &HttpApiState, addr_str: &str) -> Result<(), String> {
    let mut peers = state.bootstrap_peers.write().await;
    peers.retain(|p| p != addr_str);
    Ok(())
}
