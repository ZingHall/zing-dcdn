use crate::cache::store::BlobStore;
use crate::cache::pinning::PinningManager;
use crate::cache::eviction::EvictionManager;
use crate::walrus::client::WalrusL3Client;
use crate::walrus::verify::BlobVerifier;
use crate::mesh::reputation::PeerReputationTable;
use crate::types::{ZingError, ZingResult, BlobResolution};
use std::sync::Arc;
use tokio::sync::RwLock;
use walrus_core::BlobId;

pub struct Resolver {
    store: Arc<RwLock<BlobStore>>,
    pinning: Arc<RwLock<PinningManager>>,
    eviction: Arc<RwLock<EvictionManager>>,
    walrus_client: Arc<WalrusL3Client>,
    verifier: Arc<BlobVerifier>,
    reputation: Arc<RwLock<PeerReputationTable>>,
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
        }
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

        // Layer 1: Try DHT for peer providers
        // In the current MVP, the P2P node is a stub.
        // Full L1 resolution will be wired during integration.
        tracing::info!(blob_id = %blob_id_hex, "L1: no peers found in DHT (stub), falling back to L3");

        // Layer 3: Fetch from Walrus
        self.resolve_from_walrus(blob_id, &blob_id_hex).await
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
            let store = self.store.read().await;
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