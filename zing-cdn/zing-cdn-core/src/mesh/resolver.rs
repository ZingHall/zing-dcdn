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
    local_peer_id: Option<PeerId>,
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
        local_peer_id: Option<PeerId>,
    ) -> Self {
        Self {
            store,
            pinning,
            eviction,
            walrus_client,
            verifier,
            reputation,
            p2p_command_tx: None,
            local_peer_id,
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
        if let Some(result) = self.try_direct_peers_range_parallel(blob_id, blob_id_hex, tx).await {
            return Some(result);
        }

        if let Some(result) = self.try_direct_peers_range(blob_id, blob_id_hex, tx).await {
            return Some(result);
        }

        if let Some(result) = self.try_direct_peers(blob_id, blob_id_hex, tx).await {
            return Some(result);
        }

        let mut dht_peers = Self::run_find_providers(tx, blob_id).await;
        tracing::debug!(?dht_peers, blob_id = %blob_id_hex, "Kad providers before self-filter");
        dht_peers.retain(|p| Some(p) != self.local_peer_id.as_ref());
        if dht_peers.is_empty() {
            for attempt in 0..3 {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let mut retry = Self::run_find_providers(tx, blob_id).await;
                tracing::debug!(?retry, attempt, "Kad retry providers before self-filter");
                retry.retain(|p| Some(p) != self.local_peer_id.as_ref());
                if !retry.is_empty() {
                    dht_peers = retry;
                    break;
                }
            }
            if dht_peers.is_empty() {
                return None;
            }
        }

        for peer_id in &dht_peers {
            let (dial_reply, dial_rx) = tokio::sync::oneshot::channel();
            if tx.send(P2pCommand::DialKadPeer { peer_id: *peer_id, addrs: None, reply: dial_reply }).await.is_ok() {
                if dial_rx.await.unwrap_or(false) {
                    tracing::info!(%peer_id, %blob_id_hex, "DHT auto-dial initiated via local address book");
                }
            }
        }

        for peer_id in &dht_peers {
            let (addr_reply, addr_rx) = tokio::sync::oneshot::channel();
            if tx.send(P2pCommand::QueryPeerAddress { target: *peer_id, reply: addr_reply }).await.is_ok() {
                if let Ok(Ok(addrs)) = tokio::time::timeout(Duration::from_secs(5), addr_rx).await {
                    if !addrs.is_empty() {
                        let (dial_reply, dial_rx) = tokio::sync::oneshot::channel();
                        if tx.send(P2pCommand::DialKadPeer { peer_id: *peer_id, addrs: Some(addrs), reply: dial_reply }).await.is_ok() {
                            if dial_rx.await.unwrap_or(false) {
                                tracing::info!(%peer_id, %blob_id_hex, "DHT auto-dial initiated via peer address query");
                            }
                        }
                    } else {
                        tracing::debug!(%peer_id, %blob_id_hex, "addr query returned empty");
                    }
                }
            }
        }

        const MAX_POLL_ITERATIONS: u64 = 30;
        const POLL_INTERVAL_MS: u64 = 100;
        let peer = {
            let mut iteration = 0u64;
            loop {
                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
                let (reply, rx) = tokio::sync::oneshot::channel();
                if tx.send(P2pCommand::GetConnectedPeers { reply }).await.is_err() {
                    break None;
                }
                let connected: Vec<PeerId> = rx.await.unwrap_or(vec![]);
                if let Some(p) = dht_peers.iter().find(|p| connected.contains(p)) {
                    break Some(*p);
                }
                iteration += 1;
                if iteration >= MAX_POLL_ITERATIONS {
                    tracing::debug!(blob_id = %blob_id_hex, "DHT auto-dial: no peer connected within {}ms", MAX_POLL_ITERATIONS * POLL_INTERVAL_MS);
                    break None;
                }
            }
        };

        let peer = match peer {
            Some(p) => p,
            None => return None,
        };

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
                tracing::warn!(blob_id = %blob_id_hex, "L1: fetch failed or timed out");
                return None;
            }
        };

        self.finalize_l1_fetch(blob_id, blob_id_hex, peer, data).await
    }

    async fn try_direct_peers(
        &self,
        blob_id: &BlobId,
        blob_id_hex: &str,
        tx: &mpsc::Sender<P2pCommand>,
    ) -> Option<ZingResult<ResolveResult>> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        if tx.send(P2pCommand::GetConnectedPeers { reply }).await.is_err() {
            return None;
        }
        let connected: Vec<PeerId> = rx.await.unwrap_or(vec![]);
        if connected.is_empty() {
            return None;
        }
        for peer in &connected {
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
                    return self.finalize_l1_fetch(
                        blob_id, blob_id_hex, *peer, data,
                    ).await;
                }
                _ => continue,
            }
        }
        None
    }

    async fn try_direct_peers_range(
        &self,
        blob_id: &BlobId,
        blob_id_hex: &str,
        tx: &mpsc::Sender<P2pCommand>,
    ) -> Option<ZingResult<ResolveResult>> {
        const RANGE_CHUNK: u64 = 256 * 1024;
        const MAX_CHUNKS: u64 = 2048;

        let (reply, rx) = tokio::sync::oneshot::channel();
        if tx.send(P2pCommand::GetConnectedPeers { reply }).await.is_err() {
            return None;
        }
        let connected: Vec<PeerId> = rx.await.unwrap_or(vec![]);
        if connected.is_empty() {
            return None;
        }

        for peer in &connected {
            let (fetch_reply, fetch_rx) = tokio::sync::oneshot::channel();
            if tx.send(P2pCommand::FetchRange {
                peer_id: *peer,
                blob_id: blob_id.0,
                offset: 0,
                length: RANGE_CHUNK,
                reply: fetch_reply,
            }).await.is_err() {
                continue;
            }

            let chunk = match tokio::time::timeout(Duration::from_secs(30), fetch_rx).await {
                Ok(Ok(Ok(data))) => data,
                _ => continue,
            };

            if chunk.len() < RANGE_CHUNK as usize {
                return self.finalize_l1_fetch(
                    blob_id, blob_id_hex, *peer, chunk,
                ).await;
            }

            let mut data = chunk;
            let mut offset = RANGE_CHUNK;

            loop {
                if offset / RANGE_CHUNK >= MAX_CHUNKS {
                    tracing::warn!(blob_id = %blob_id_hex, chunks = offset / RANGE_CHUNK, "L1 range: exceeded max chunks, aborting");
                    return None;
                }

                let (next_reply, next_rx) = tokio::sync::oneshot::channel();
                if tx.send(P2pCommand::FetchRange {
                    peer_id: *peer,
                    blob_id: blob_id.0,
                    offset,
                    length: RANGE_CHUNK,
                    reply: next_reply,
                }).await.is_err() {
                    break;
                }

                let next_chunk = match tokio::time::timeout(Duration::from_secs(30), next_rx).await {
                    Ok(Ok(Ok(d))) => d,
                    _ => break,
                };

                if next_chunk.is_empty() {
                    break;
                }

                data.extend_from_slice(&next_chunk);
                offset += RANGE_CHUNK;

                if next_chunk.len() < RANGE_CHUNK as usize {
                    break;
                }
            }

            return self.finalize_l1_fetch(
                blob_id, blob_id_hex, *peer, data,
            ).await;
        }

        None
    }

    async fn try_direct_peers_range_parallel(
        &self,
        blob_id: &BlobId,
        blob_id_hex: &str,
        tx: &mpsc::Sender<P2pCommand>,
    ) -> Option<ZingResult<ResolveResult>> {
        const CHUNK_SIZE: u64 = 64 * 1024;
        const MAX_PARALLEL: usize = 5;
        const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
        const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
        const MAX_TOTAL_SIZE: usize = 512 * 1024 * 1024;

        let (reply, rx) = tokio::sync::oneshot::channel();
        if tx.send(P2pCommand::GetConnectedPeers { reply }).await.is_err() {
            return None;
        }
        let connected: Vec<PeerId> = rx.await.unwrap_or(vec![]);
        if connected.is_empty() {
            return None;
        }

        let mut probe_futs = Vec::new();
        for peer in connected.iter().take(MAX_PARALLEL) {
            let (fetch_reply, fetch_rx) = tokio::sync::oneshot::channel();
            if tx.send(P2pCommand::FetchRange {
                peer_id: *peer,
                blob_id: blob_id.0,
                offset: 0,
                length: 8,
                reply: fetch_reply,
            }).await.is_err() {
                continue;
            }
            probe_futs.push((*peer, tokio::time::timeout(PROBE_TIMEOUT, fetch_rx)));
        }

        let probe_results = futures::future::join_all(
            probe_futs.into_iter().map(|(peer, fut)| async move {
                match fut.await {
                    Ok(Ok(Ok(data))) if !data.is_empty() => Some(peer),
                    _ => None,
                }
            })
        ).await;

        let working: Vec<PeerId> = probe_results.into_iter().flatten().collect();
        if working.is_empty() {
            return None;
        }
        tracing::info!(
            blob_id = %blob_id_hex,
            peers = working.len(),
            "L1 range parallel: found peers with blob"
        );

        let n = working.len();
        let mut data: Vec<u8> = Vec::new();

        for window in 0u64.. {
            let base_offset = window * n as u64 * CHUNK_SIZE;
            let mut fetch_futs = Vec::new();

            for (i, peer) in working.iter().enumerate() {
                let offset = base_offset + i as u64 * CHUNK_SIZE;
                let (fetch_reply, fetch_rx) = tokio::sync::oneshot::channel();
                if tx.send(P2pCommand::FetchRange {
                    peer_id: *peer,
                    blob_id: blob_id.0,
                    offset,
                    length: CHUNK_SIZE,
                    reply: fetch_reply,
                }).await.is_err() {
                    continue;
                }
                fetch_futs.push((offset, tokio::time::timeout(FETCH_TIMEOUT, fetch_rx)));
            }

            if fetch_futs.is_empty() {
                break;
            }

            let responses = futures::future::join_all(
                fetch_futs.into_iter().map(|(offset, fut)| async move {
                    match fut.await {
                        Ok(Ok(Ok(d))) if !d.is_empty() => Some((offset, d)),
                        _ => None,
                    }
                })
            ).await;

            let mut chunks: Vec<(u64, Vec<u8>)> = responses.into_iter().flatten().collect();
            if chunks.is_empty() {
                break;
            }

            chunks.sort_by_key(|(o, _)| *o);

            let mut expected: u64 = data.len() as u64;
            let mut has_gap = false;
            let mut eof = false;
            for (offset, chunk) in chunks {
                if offset != expected {
                    tracing::warn!(blob_id = %blob_id_hex, expected, actual = offset, "range parallel: gap detected, aborting window");
                    has_gap = true;
                    break;
                }
                expected = offset + chunk.len() as u64;
                if chunk.len() < CHUNK_SIZE as usize {
                    eof = true;
                }
                data.extend_from_slice(&chunk);
            }
            if has_gap {
                break;
            }

            if data.len() > MAX_TOTAL_SIZE {
                tracing::warn!(blob_id = %blob_id_hex, size = data.len(), "L1 range parallel: exceeded max size, aborting");
                return None;
            }

            if eof {
                let peer = working[0];
                return self.finalize_l1_fetch(
                    blob_id, blob_id_hex, peer, data,
                ).await;
            }
        }

        if data.is_empty() {
            return None;
        }

        let peer = working[0];
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
        let peers: Vec<PeerId> = tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .unwrap_or(Ok(vec![]))
            .unwrap_or(vec![]);

        tracing::info!(
            blob_id = %blob_id,
            providers = peers.len(),
            "Kad GetProviders"
        );

        peers
    }

    async fn finalize_l1_fetch(
        &self,
        blob_id: &BlobId,
        blob_id_hex: &str,
        peer: PeerId,
        data: Vec<u8>,
    ) -> Option<ZingResult<ResolveResult>> {
        if let Err(e) = self.verifier.verify_blob_by_id(blob_id, &data) {
            tracing::warn!(blob_id = %blob_id_hex, error = %e, "L1: verification failed");
            self.reputation.write().await.record_corruption(&peer.to_string());
            return None;
        }

        {
            let store = self.store.write().await;
            if let Err(e) = store.put(blob_id_hex, &data) {
                tracing::warn!(blob_id = %blob_id_hex, error = %e, "L1: cache write failed");
            }
        }
        {
            let pinning = self.pinning.read().await;
            let _ = self.eviction.write().await.run(&pinning);
        }

        self.reputation.write().await.record_success(&peer.to_string());
        tracing::info!(blob_id = %blob_id_hex, peer = %peer, data_len = data.len(), "L1: blob verified and cached from peer");

        Some(Ok(ResolveResult {
            data,
            resolution: BlobResolution::L1Peer,
            cached: true,
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
            cached: true,
        })
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
