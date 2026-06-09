use crate::cache::store::BlobStore;
use crate::cache::pinning::PinningManager;
use crate::cache::eviction::EvictionManager;
use crate::walrus::client::WalrusL3Client;
use crate::walrus::verify::BlobVerifier;
use crate::mesh::reputation::PeerReputationTable;
use crate::types::{ZingResult, BlobResolution};
use std::sync::Arc;
use std::time::Duration;
use libp2p::PeerId;
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

        // Layer 1: P2P peers (no metadata needed upfront)
        if let Some(ref tx) = self.p2p_command_tx {
            if let Some(result) = self.resolve_from_l1(blob_id, &blob_id_hex, tx).await {
                return result;
            }
        }

        // Layer 3: Fetch from Walrus
        self.resolve_from_walrus(blob_id, &blob_id_hex).await
    }

    async fn resolve_from_l1(
        &self,
        blob_id: &BlobId,
        blob_id_hex: &str,
        tx: &mpsc::Sender<P2pCommand>,
    ) -> Option<ZingResult<ResolveResult>> {
        eprintln!("L1: looking up DHT providers for {blob_id_hex}");

        let peers = Self::run_find_providers(tx, blob_id).await;

        let peers = if peers.is_empty() {
            eprintln!("L1: DHT returned empty, retrying in 2s...");
            tokio::time::sleep(Duration::from_secs(2)).await;
            let retry = Self::run_find_providers(tx, blob_id).await;
            if retry.is_empty() {
                eprintln!("L1: retry also empty, trying direct connected peers");
                let (reply, rx) = tokio::sync::oneshot::channel();
                if tx.send(P2pCommand::GetConnectedPeers { reply }).await.is_err() {
                    return None;
                }
                let connected: Vec<PeerId> = rx.await.unwrap_or(vec![]);
                eprintln!("L1: {} connected peer(s) available", connected.len());
                if connected.is_empty() {
                    return None;
                }
                for peer in &connected {
                    eprintln!("L1: trying direct FetchBlob from {peer}");
                    let (fetch_reply, fetch_rx) = tokio::sync::oneshot::channel();
                    if tx.send(P2pCommand::FetchBlob {
                        peer_id: *peer,
                        blob_id: blob_id.0,
                        reply: fetch_reply,
                    }).await.is_err() {
                        continue;
                    }
                    match tokio::time::timeout(Duration::from_secs(30), fetch_rx).await {
                        Ok(Ok(Ok(data))) => {
                            eprintln!("L1: got blob from direct peer {peer}");
                            return self.finalize_l1_fetch(
                                blob_id, blob_id_hex, *peer, data,
                            ).await;
                        }
                        _ => {
                            eprintln!("L1: direct fetch from {peer} failed, trying next");
                        }
                    }
                }
                return None;
            }
            retry
        } else {
            peers
        };

        if peers.is_empty() {
            eprintln!("L1: no providers found in DHT for {blob_id_hex}");
            return None;
        }

        let peer = {
            let rep = self.reputation.read().await;
            peers.into_iter()
                .max_by_key(|p| rep.get_score(&p.to_string()).unwrap_or(0))?
        };

        eprintln!("L1: fetching blob {blob_id_hex} from peer {peer}");

        let (fetch_reply, fetch_rx) = tokio::sync::oneshot::channel();
        if tx.send(P2pCommand::FetchBlob {
            peer_id: peer,
            blob_id: blob_id.0,
            reply: fetch_reply,
        }).await.is_err() {
            return None;
        }

        let data = match tokio::time::timeout(Duration::from_secs(30), fetch_rx).await {
            Ok(Ok(Ok(data))) => data,
            _ => {
                eprintln!("L1: fetch failed or timed out for {blob_id_hex}");
                return None;
            }
        };

        self.finalize_l1_fetch(blob_id, blob_id_hex, peer, data).await
    }

    async fn run_find_providers(
        tx: &mpsc::Sender<P2pCommand>,
        blob_id: &BlobId,
    ) -> Vec<PeerId> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        if tx.send(P2pCommand::FindProviders {
            blob_id: blob_id.0,
            reply,
        }).await.is_err() {
            return vec![];
        }
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .unwrap_or(Ok(vec![]))
            .unwrap_or(vec![])
    }

    async fn finalize_l1_fetch(
        &self,
        blob_id: &BlobId,
        blob_id_hex: &str,
        peer: PeerId,
        data: Vec<u8>,
    ) -> Option<ZingResult<ResolveResult>> {
        let metadata = match self.walrus_client.fetch_metadata(blob_id).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(blob_id = %blob_id_hex, error = %e, "L1: metadata fetch failed");
                eprintln!("L1: metadata fetch failed: {e}");
                return None;
            }
        };

        if let Err(e) = self.verifier.verify_blob_against_metadata(&metadata, &data) {
            tracing::warn!(blob_id = %blob_id_hex, error = %e, "L1: verification failed");
            eprintln!("L1: verification failed: {e}");
            self.reputation.write().await.record_corruption(&peer.to_string());
            return None;
        }

        {
            let store = self.store.write().await;
            if let Err(e) = store.put(blob_id_hex, &data) {
                eprintln!("L1: cache write failed: {e}");
            }
        }
        {
            let pinning = self.pinning.read().await;
            let _ = self.eviction.write().await.run(&pinning);
        }

        self.reputation.write().await.record_success(&peer.to_string());
        eprintln!("L1: SUCCESS — blob fetched and verified from peer {peer}");

        Some(Ok(ResolveResult {
            data,
            resolution: BlobResolution::L1Peer,
            cached: false,
        }))
    }

    async fn resolve_from_walrus(&self, blob_id: &BlobId, blob_id_hex: &str) -> ZingResult<ResolveResult> {
        tracing::info!(blob_id = %blob_id_hex, "L3: fetching blob from Walrus storage nodes");
        let data = self.walrus_client.read_blob(blob_id).await?;
        let size = data.len();

        tracing::info!(blob_id = %blob_id_hex, size = size, "L3: blob verified, caching locally");

        {
            let store = self.store.write().await;
            store.put(blob_id_hex, &data)?;
        }
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
