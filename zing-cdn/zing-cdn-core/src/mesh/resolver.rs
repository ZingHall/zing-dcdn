use crate::cache::store::BlobStore;
use crate::cache::pinning::PinningManager;
use crate::cache::eviction::EvictionManager;
use crate::walrus::client::WalrusL3Client;
use crate::walrus::verify::BlobVerifier;
use crate::mesh::reputation::PeerReputationTable;
use crate::types::{ZingError, ZingResult, BlobResolution};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use walrus_core::BlobId;

use crate::p2p::node::P2pCommand;

pub struct Resolver {
    store: Arc<RwLock<BlobStore>>,
    pinning: Arc<RwLock<PinningManager>>,
    eviction: Arc<RwLock<EvictionManager>>,
    walrus_client: Arc<WalrusL3Client>,
    verifier: Arc<BlobVerifier>,
    reputation: Arc<RwLock<PeerReputationTable>>,
    p2p_command_tx: Option<mpsc::Sender<P2pCommand>>,
}

#[derive(Debug, Clone)]
pub struct ResolveResult {
    pub data: Vec<u8>,
    pub resolution: BlobResolution,
    pub cached: bool,
}

impl Resolver {
    pub fn new(
        store: Arc<RwLock<BlobStore>>,
        pinning: Arc<RwLock<PinningManager>>,
        eviction: Arc<RwLock<EvictionManager>>,
        walrus_client: Arc<WalrusL3Client>,
        verifier: Arc<BlobVerifier>,
        reputation: Arc<RwLock<PeerReputationTable>>,
    ) -> Self {
        Self {
            store,
            pinning,
            eviction,
            walrus_client,
            verifier,
            reputation,
            p2p_command_tx: None,
        }
    }

    pub fn set_p2p_channel(&mut self, tx: mpsc::Sender<P2pCommand>) {
        self.p2p_command_tx = Some(tx);
    }

    pub async fn resolve(&self, blob_id: &BlobId) -> ZingResult<ResolveResult> {
        let blob_id_hex = blob_id.to_string();
        tracing::info!(blob_id = %blob_id_hex, "resolving blob request");

        // Layer 0: Local cache
        {
            let store = self.store.read().await;
            if let Some(data) = store.get(&blob_id_hex)? {
                tracing::info!(blob_id = %blob_id_hex, "L0: blob found in local cache");
                return Ok(ResolveResult {
                    data,
                    resolution: BlobResolution::LocalCache,
                    cached: true,
                });
            }
        }

        // Layer 0.5: Metadata pre-fetch from Walrus for L1 verification
        tracing::info!(blob_id = %blob_id_hex, "fetching metadata for verification");
        let metadata = match self.walrus_client.fetch_metadata(blob_id).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(blob_id = %blob_id_hex, error = %e, "metadata pre-fetch failed, forcing L3");
                return self.resolve_from_walrus(blob_id, &blob_id_hex).await;
            }
        };

        // Layer 1: P2P peers
        if let Some(ref tx) = self.p2p_command_tx {
            if let Ok(data) = self.resolve_from_l1(blob_id, &blob_id_hex, &metadata, tx).await {
                return Ok(data);
            }
        }

        // Layer 3: Fetch from Walrus
        self.resolve_from_walrus(blob_id, &blob_id_hex).await
    }

    async fn resolve_from_l1(
        &self,
        blob_id: &BlobId,
        blob_id_hex: &str,
        metadata: &walrus_core::metadata::VerifiedBlobMetadataWithId,
        tx: &mpsc::Sender<P2pCommand>,
    ) -> ZingResult<ResolveResult> {
        tracing::info!(blob_id = %blob_id_hex, "L1: looking up DHT providers");

        let (find_reply, find_rx) = tokio::sync::oneshot::channel();
        tx.send(P2pCommand::FindProviders {
            blob_id: blob_id.0,
            reply: find_reply,
        }).await.map_err(|_| ZingError::P2PNetwork("p2p channel closed".into()))?;

        let peers = tokio::time::timeout(Duration::from_secs(5), find_rx)
            .await
            .unwrap_or(Ok(vec![]))
            .unwrap_or(vec![]);

        if peers.is_empty() {
            tracing::info!(blob_id = %blob_id_hex, "L1: no providers found in DHT");
            return Err(ZingError::NoPeersAvailable(blob_id_hex.to_string()));
        }

        // Pick highest-reputation peer
        let peer = {
            let reputation = self.reputation.read().await;
            peers.into_iter()
                .max_by_key(|p| reputation.get_score(&p.to_string()).unwrap_or(0))
                .ok_or_else(|| ZingError::NoPeersAvailable(blob_id_hex.to_string()))?
        };

        tracing::info!(blob_id = %blob_id_hex, peer = %peer, "L1: fetching from peer");

        let (fetch_reply, fetch_rx) = tokio::sync::oneshot::channel();
        tx.send(P2pCommand::FetchBlob {
            peer_id: peer,
            blob_id: blob_id.0,
            reply: fetch_reply,
        }).await.map_err(|_| ZingError::P2PNetwork("p2p channel closed".into()))?;

        let data = tokio::time::timeout(Duration::from_secs(30), fetch_rx)
            .await
            .map_err(|_| ZingError::P2PNetwork("fetch timeout".into()))?
            .map_err(|_| ZingError::P2PNetwork("fetch channel closed".into()))?
            ?;

        // Verify blob against metadata
        self.verifier.verify_blob_against_metadata(metadata, &data)?;

        // Cache locally
        {
            let store = self.store.write().await;
            store.put(blob_id_hex, &data)?;
        }
        {
            let pinning = self.pinning.read().await;
            self.eviction.write().await.run(&pinning)?;
        }

        // Record success
        self.reputation.write().await.record_success(&peer.to_string());

        tracing::info!(blob_id = %blob_id_hex, peer = %peer, "L1: blob fetched and verified");
        Ok(ResolveResult {
            data,
            resolution: BlobResolution::L1Peer,
            cached: false,
        })
    }

    async fn resolve_from_walrus(&self, blob_id: &BlobId, blob_id_hex: &str) -> ZingResult<ResolveResult> {
        tracing::info!(blob_id = %blob_id_hex, "L3: fetching blob from Walrus storage nodes");
        let data = self.walrus_client.read_blob(blob_id).await?;
        let size = data.len();

        tracing::info!(blob_id = %blob_id_hex, size = size, "L3: blob verified, caching locally");

        // Cache the blob
        {
            let store = self.store.write().await;
            store.put(blob_id_hex, &data)?;
        }

        // Run eviction to stay within budget
        {
            let pinning = self.pinning.read().await;
            self.eviction.write().await.run(&pinning)?;
        }

        Ok(ResolveResult {
            data,
            resolution: BlobResolution::L3Walrus,
            cached: false,
        })
    }

    pub fn verify_l1_blob(&self, metadata: &walrus_core::metadata::VerifiedBlobMetadataWithId, data: &[u8]) -> ZingResult<()> {
        self.verifier.verify_blob_against_metadata(metadata, data)
    }

    pub async fn record_peer_success(&self, peer_id: &str) {
        self.reputation.write().await.record_success(peer_id);
    }

    pub async fn record_peer_corruption(&self, peer_id: &str) {
        self.reputation.write().await.record_corruption(peer_id);
    }

    pub async fn record_peer_dropped(&self, peer_id: &str) {
        self.reputation.write().await.record_dropped(peer_id);
    }

    pub async fn record_peer_false_claim(&self, peer_id: &str) {
        self.reputation.write().await.record_false_claim(peer_id);
    }
}