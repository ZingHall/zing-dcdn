use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use libp2p::{PeerId, Multiaddr};
use serde::Serialize;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::P2pCommand;

#[derive(Clone)]
pub struct HttpApiState {
    pub store: Arc<RwLock<BlobStore>>,
    pub pinning: Arc<RwLock<PinningManager>>,
    pub eviction: Arc<RwLock<EvictionManager>>,
    pub p2p_tx: mpsc::Sender<P2pCommand>,
    pub peer_id: PeerId,
    pub listen_addr: Multiaddr,
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
    use zing_cdn_core::client::ZingClient;
    use zing_cdn_core::mesh::resolver::Resolver;
    use zing_cdn_core::mesh::reputation::PeerReputationTable;
    use zing_cdn_core::walrus::verify::BlobVerifier;
    use walrus_core::BlobId;

    let id: BlobId = blob_id.parse().map_err(|e| format!("invalid blob id: {blob_id}: {e}"))?;

    let client = ZingClient::from_mainnet().await.map_err(|e| e.to_string())?;
    let verifier = Arc::new(BlobVerifier::new(client.encoding_config_arc()));

    let mut resolver = Resolver::new(
        state.store.clone(),
        state.pinning.clone(),
        state.eviction.clone(),
        client.walrus_client_arc(),
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
        let preview_size = data.len().min(512 * 1024); // cap at 512KB
        (
            format!("[Binary image — {} bytes]", data.len()),
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &data[..preview_size],
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
