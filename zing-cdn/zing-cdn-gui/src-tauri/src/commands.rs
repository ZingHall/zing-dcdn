use std::sync::Arc;
use tokio::sync::{RwLock, oneshot};
use tauri::State;

use crate::state::AppState;

use zing_cdn_core::mesh::resolver::Resolver;
use zing_cdn_core::mesh::reputation::PeerReputationTable;
use zing_cdn_core::walrus::verify::BlobVerifier;
use zing_cdn_core::client::ZingClient;
use zing_cdn_core::p2p::P2pCommand;
use walrus_core::BlobId;

#[derive(serde::Serialize)]
pub struct BlobInfo {
    pub blob_id: String,
    pub size: u64,
    pub source: String,
    pub cached: bool,
}

#[derive(serde::Serialize)]
pub struct CacheEntry {
    pub blob_id: String,
    pub size: u64,
    pub pinned: bool,
}

#[derive(serde::Serialize)]
pub struct DashboardInfo {
    pub peer_id: String,
    pub listen_addr: String,
    pub connected_peers: Vec<String>,
    pub cache_used: u64,
    pub cache_budget: u64,
    pub cache_count: usize,
}

struct ResolveSession {
    _client: ZingClient,
    resolver: Resolver,
}

async fn get_resolver(state: &State<'_, AppState>) -> Result<ResolveSession, String> {
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
    Ok(ResolveSession { _client: client, resolver })
}

#[tauri::command]
pub async fn resolve_blob(blob_id: String, state: State<'_, AppState>) -> Result<BlobInfo, String> {
    let id: BlobId = blob_id.parse().map_err(|e| format!("invalid blob id: {blob_id}: {e}"))?;
    let session = get_resolver(&state).await?;
    let result = session.resolver.resolve(&id).await.map_err(|e| e.to_string())?;
    let source = match result.resolution {
        zing_cdn_core::types::BlobResolution::LocalCache => "L0 local cache",
        zing_cdn_core::types::BlobResolution::L1Peer => "L1 peer",
        zing_cdn_core::types::BlobResolution::L3Walrus => "L3 Walrus",
    };
    Ok(BlobInfo {
        blob_id,
        size: result.data.len() as u64,
        source: source.to_string(),
        cached: result.cached,
    })
}

#[tauri::command]
pub async fn get_blob_content(blob_id: String, state: State<'_, AppState>) -> Result<Vec<u8>, String> {
    {
        let store = state.store.read().await;
        if let Some(data) = store.get(&blob_id).map_err(|e| e.to_string())? {
            return Ok(data);
        }
    }
    let id: BlobId = blob_id.parse().map_err(|e| format!("invalid blob id: {blob_id}: {e}"))?;
    let session = get_resolver(&state).await?;
    let result = session.resolver.resolve(&id).await.map_err(|e| e.to_string())?;
    Ok(result.data)
}

#[tauri::command]
pub async fn list_cache(state: State<'_, AppState>) -> Result<Vec<CacheEntry>, String> {
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

#[tauri::command]
pub async fn pin_blob(blob_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.pinning.read().await.pin(&blob_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unpin_blob(blob_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.pinning.read().await.unpin(&blob_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_blob(blob_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.store.write().await.delete(&blob_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_dashboard_info(state: State<'_, AppState>) -> Result<DashboardInfo, String> {
    let store = state.store.read().await;
    let ids = store.list_blob_ids().map_err(|e| e.to_string())?;
    let cache_count = ids.len();
    let cache_used = store.total_size().map_err(|e| e.to_string())?;

    let (reply, rx) = oneshot::channel();
    state.p2p_tx.send(P2pCommand::GetConnectedPeers { reply }).await.map_err(|e| e.to_string())?;
    let connected: Vec<libp2p::PeerId> = rx.await.map_err(|e| e.to_string())?;

    Ok(DashboardInfo {
        peer_id: state.peer_id.to_string(),
        listen_addr: state.listen_addr.to_string(),
        connected_peers: connected.iter().map(|p| p.to_string()).collect(),
        cache_used,
        cache_budget: 500 * 1024 * 1024,
        cache_count,
    })
}
