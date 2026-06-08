use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use libp2p::{PeerId, Multiaddr};
use serde::Serialize;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::p2p::P2pCommand;

#[derive(Clone)]
pub struct HttpApiState {
    pub store: Arc<RwLock<BlobStore>>,
    pub pinning: Arc<RwLock<PinningManager>>,
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
