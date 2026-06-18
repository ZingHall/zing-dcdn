use std::collections::HashMap;
use std::time::Duration;
use futures::stream::StreamExt;
use libp2p::identity;
use libp2p::kad;
use libp2p::request_response;
use libp2p::core::ConnectedPoint;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::p2p::behaviour::ZingBehaviour;
use crate::p2p::behaviour::ZingBehaviourEvent;
use crate::p2p::handler::{handle_inbound_range_request, handle_inbound_sliver_request, handle_inbound_request, BlobStoreHandle};
use crate::p2p::protocol::{BlobRequest, RangeRequest, SliverRequest, AddrRequest, AddrResponse};
use crate::types::{ZingError, ZingResult};
use walrus_core::BlobId;

#[derive(Debug)]
pub enum P2pCommand {
    AnnounceBlob { blob_id: [u8; 32] },
    FindProviders {
        blob_id: [u8; 32],
        reply: oneshot::Sender<Vec<PeerId>>,
    },
    FetchBlob {
        peer_id: PeerId,
        blob_id: [u8; 32],
        payment_tx_digest: [u8; 32],
        reply: oneshot::Sender<ZingResult<Vec<u8>>>,
    },
    FetchRange {
        peer_id: PeerId,
        blob_id: [u8; 32],
        offset: u64,
        length: u64,
        payment_tx_digest: [u8; 32],
        reply: oneshot::Sender<ZingResult<Vec<u8>>>,
    },
    FetchSliver {
        peer_id: PeerId,
        blob_id: [u8; 32],
        sliver_pair_index: u16,
        axis: u8,
        reply: oneshot::Sender<ZingResult<Vec<u8>>>,
    },
    DialKadPeer {
        peer_id: PeerId,
        addrs: Option<Vec<Multiaddr>>,
        reply: oneshot::Sender<bool>,
    },
    AddBootstrapPeer {
        peer_id: PeerId,
        addr: Multiaddr,
    },
    Bootstrap,
    GetConnectedPeers { reply: oneshot::Sender<Vec<PeerId>> },
    Dial { peer_id: PeerId, addr: Multiaddr },
    Disconnect { peer_id: PeerId },
    QueryPeerAddress {
        target: PeerId,
        reply: oneshot::Sender<Vec<Multiaddr>>,
    },
    GetPeerSuiAddress {
        peer_id: PeerId,
        reply: oneshot::Sender<Option<[u8; 32]>>,
    },
    GetPeerVault {
        peer_id: PeerId,
        reply: oneshot::Sender<Option<[u8; 32]>>,
    },
}

pub struct ZingP2pNode {
    key: identity::Keypair,
    local_peer_id: PeerId,
    command_tx: mpsc::Sender<P2pCommand>,
}

impl ZingP2pNode {
    pub fn new(_store: BlobStoreHandle, key: identity::Keypair) -> (Self, mpsc::Receiver<P2pCommand>) {
        let local_peer_id = key.public().to_peer_id();
        let (command_tx, command_rx) = mpsc::channel(256);
        (Self { key, local_peer_id, command_tx }, command_rx)
    }

    pub fn key(&self) -> &identity::Keypair {
        &self.key
    }

    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    pub fn command_tx(&self) -> &mpsc::Sender<P2pCommand> {
        &self.command_tx
    }

    pub async fn run(
        key: identity::Keypair,
        mut command_rx: mpsc::Receiver<P2pCommand>,
        store: BlobStoreHandle,
        listen_addr: Multiaddr,
        bootstrap_addrs: Vec<(PeerId, Multiaddr)>,
        external_addrs: Vec<Multiaddr>,
        sui_address: Option<[u8; 32]>,
        vault_object_id: Option<[u8; 32]>,
    ) -> ZingResult<()> {

        let store_for_builder = store.clone();
        let mut swarm = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_quic()
            .with_behaviour(move |key| ZingBehaviour::new(key, store_for_builder))
            .map_err(|e| ZingError::P2PNetwork(e.to_string()))?
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(300)))
            .build();

        swarm
            .listen_on(listen_addr)
            .map_err(|e| ZingError::P2PNetwork(e.to_string()))?;

        swarm.behaviour_mut().kad.set_mode(Some(kad::Mode::Server));

        // Register external addresses so that `start_providing` includes real
        // dialable addresses in ADD_PROVIDER messages. Without this, provider
        // records are stored on remote peers with empty `addresses`, forcing
        // `provider_peers()` to fall back to the routing table.
        for addr in &external_addrs {
            swarm.add_external_address(addr.clone());
            tracing::info!(%addr, "Registered external address for Kad provider records");
        }

        let sui_addr_to_publish = sui_address;
        let vault_id_to_publish = vault_object_id;

        for (peer_id, addr) in &bootstrap_addrs {
            swarm.behaviour_mut().kad.add_address(peer_id, addr.clone());
        }

        for (peer_id, addr) in &bootstrap_addrs {
            let mut dial_addr = addr.clone();
            dial_addr.push(libp2p::multiaddr::Protocol::P2p(*peer_id));
            match swarm.dial(dial_addr) {
                Ok(()) => tracing::info!(%peer_id, "dialing bootstrap peer"),
                Err(e) => tracing::warn!(%peer_id, error = %e, "bootstrap dial failed"),
            }
        }

        let mut pending_finds: HashMap<kad::QueryId, oneshot::Sender<Vec<PeerId>>> = HashMap::new();
        let mut pending_fetches: HashMap<
            request_response::OutboundRequestId,
            oneshot::Sender<ZingResult<Vec<u8>>>,
        > = HashMap::new();
        let mut pending_range_fetches: HashMap<
            request_response::OutboundRequestId,
            oneshot::Sender<ZingResult<Vec<u8>>>,
        > = HashMap::new();
        let mut pending_sliver_fetches: HashMap<
            request_response::OutboundRequestId,
            oneshot::Sender<ZingResult<Vec<u8>>>,
        > = HashMap::new();
        let mut pending_addr_queries: HashMap<
            request_response::OutboundRequestId,
            oneshot::Sender<Vec<Multiaddr>>,
        > = HashMap::new();
        let mut peer_addresses: HashMap<PeerId, Vec<Multiaddr>> = HashMap::new();
        let mut bootstrap_peers: std::collections::HashSet<PeerId> = bootstrap_addrs.iter().map(|(pid, _)| *pid).collect();
        let mut bootstrap_done = false;
        let mut sui_addr_published = false;
        let mut vault_published = false;
        let mut pending_announces: Vec<[u8; 32]> = Vec::new();
        let mut announce_retried: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
        let mut pending_sui_addr_queries: HashMap<kad::QueryId, oneshot::Sender<Option<[u8; 32]>>> = HashMap::new();
        let mut pending_vault_queries: HashMap<kad::QueryId, oneshot::Sender<Option<[u8; 32]>>> = HashMap::new();
        let mut republish_sui_interval = tokio::time::interval(Duration::from_secs(3600));

        loop {
            tokio::select! {
                Some(cmd) = command_rx.recv() => {
                    Self::handle_command(
                        &mut swarm,
                        cmd,
                        &mut pending_finds,
                        &mut pending_fetches,
                        &mut pending_range_fetches,
                        &mut pending_sliver_fetches,
                        &mut pending_addr_queries,
                        &mut peer_addresses,
                        &mut pending_announces,
                        &mut pending_sui_addr_queries,
                        &mut pending_vault_queries,
                    );
                }
                event = swarm.next() => {
                    match event {
                        Some(event) => Self::handle_swarm_event(
                            &mut swarm,
                            event,
                            &mut pending_finds,
                            &mut pending_fetches,
                            &mut pending_range_fetches,
                            &mut pending_sliver_fetches,
                            &mut pending_addr_queries,
                            &mut peer_addresses,
                            &mut bootstrap_peers,
                            &mut bootstrap_done,
                            &mut pending_announces,
                            &mut announce_retried,
                            &mut pending_sui_addr_queries,
                            &mut pending_vault_queries,
                            sui_addr_to_publish,
                            vault_id_to_publish,
                            &mut sui_addr_published,
                            &mut vault_published,
                            &store,
                        ).await,
                        None => break,
                    }
                }
                _ = republish_sui_interval.tick() => {
                    if let Some(sui_addr) = sui_addr_to_publish {
                        publish_sui_addr_record(&mut swarm, sui_addr);
                    }
                    if let Some(vault_id) = vault_id_to_publish {
                        publish_vault_record(&mut swarm, vault_id);
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_command(
        swarm: &mut Swarm<ZingBehaviour>,
        cmd: P2pCommand,
        pending_finds: &mut HashMap<kad::QueryId, oneshot::Sender<Vec<PeerId>>>,
        pending_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        pending_range_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        pending_sliver_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        pending_addr_queries: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<Vec<Multiaddr>>>,
        peer_addresses: &HashMap<PeerId, Vec<Multiaddr>>,
        pending_announces: &mut Vec<[u8; 32]>,
        pending_sui_addr_queries: &mut HashMap<kad::QueryId, oneshot::Sender<Option<[u8; 32]>>>,
        pending_vault_queries: &mut HashMap<kad::QueryId, oneshot::Sender<Option<[u8; 32]>>>,
    ) {
        match cmd {
            P2pCommand::AnnounceBlob { blob_id } => {
                let key = kad::RecordKey::new(&blob_id);
                if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
                    tracing::warn!(error = %e, "Kad start_providing failed");
                }
                let connected_count = swarm.connected_peers().count();
                if connected_count == 0 {
                    tracing::info!(blob_id = %hex::encode(blob_id), "No connected peers, queueing announce for retry");
                    pending_announces.push(blob_id);
                } else {
                    tracing::info!(blob_id = %hex::encode(blob_id), connected_peers = connected_count, "Kad start_providing dispatched");
                }
            }
            P2pCommand::FindProviders { blob_id, reply } => {
                let key = kad::RecordKey::new(&blob_id);
                let query_id = swarm.behaviour_mut().kad.get_providers(key);
                pending_finds.insert(query_id, reply);
            }
            P2pCommand::FetchBlob {
                peer_id,
                blob_id,
                payment_tx_digest,
                reply,
            } => {
                let request = BlobRequest {
                    blob_id,
                    version: 0,
                    payment_tx_digest,
                };
                let request_id = swarm
                    .behaviour_mut()
                    .data
                    .send_request(&peer_id, request);
                pending_fetches.insert(request_id, reply);
            }
            P2pCommand::FetchRange {
                peer_id,
                blob_id,
                offset,
                length,
                payment_tx_digest,
                reply,
            } => {
                let request = RangeRequest {
                    blob_id,
                    offset,
                    length,
                    payment_tx_digest,
                };
                let request_id = swarm
                    .behaviour_mut()
                    .range
                    .send_request(&peer_id, request);
                pending_range_fetches.insert(request_id, reply);
            }
            P2pCommand::FetchSliver {
                peer_id,
                blob_id,
                sliver_pair_index,
                axis,
                reply,
            } => {
                let request = SliverRequest {
                    blob_id,
                    sliver_pair_index,
                    axis,
                };
                let request_id = swarm
                    .behaviour_mut()
                    .sliver
                    .send_request(&peer_id, request);
                pending_sliver_fetches.insert(request_id, reply);
            }
            P2pCommand::DialKadPeer { peer_id, addrs, reply } => {
                let addresses = match addrs {
                    Some(explicit) if !explicit.is_empty() => explicit,
                    _ => peer_addresses.get(&peer_id).cloned().unwrap_or(vec![]),
                };

                let success = if !addresses.is_empty() {
                    let mut connected = false;
                    for addr in &addresses {
                        let mut dial_addr = addr.clone();
                        if !addr.iter().any(|p| matches!(p, libp2p::multiaddr::Protocol::P2p(_))) {
                            dial_addr.push(libp2p::multiaddr::Protocol::P2p(peer_id));
                        }
                        if swarm.dial(dial_addr).is_ok() {
                            connected = true;
                            tracing::info!(%peer_id, %addr, "dialing Kad-discovered peer");
                        }
                    }
                    connected
                } else {
                    false
                };
                let _ = reply.send(success);
            }
            P2pCommand::QueryPeerAddress { target, reply } => {
                let connected: Vec<PeerId> = swarm.connected_peers().copied().collect();
                let mut reply_opt = Some(reply);
                for peer_id in &connected {
                    let request = AddrRequest { peer_id: target };
                    let request_id = swarm
                        .behaviour_mut()
                        .addr
                        .send_request(peer_id, request);
                    pending_addr_queries.insert(request_id, reply_opt.take().unwrap());
                    break;
                }
                if let Some(reply) = reply_opt {
                    let _ = reply.send(vec![]);
                }
            }
            P2pCommand::AddBootstrapPeer { peer_id, addr } => {
                swarm.behaviour_mut().kad.add_address(&peer_id, addr);
            }
            P2pCommand::Bootstrap => {
                if let Err(e) = swarm.behaviour_mut().kad.bootstrap() {
                    tracing::warn!(error = %e, "Kad bootstrap failed");
                }
            }
            P2pCommand::GetConnectedPeers { reply } => {
                let peers: Vec<PeerId> = swarm.connected_peers().copied().collect();
                let _ = reply.send(peers);
            }
            P2pCommand::Dial { peer_id, addr } => {
                let mut dial_addr = addr.clone();
                dial_addr.push(libp2p::multiaddr::Protocol::P2p(peer_id));
                match swarm.dial(dial_addr) {
                    Ok(()) => tracing::info!(%peer_id, "dialing peer"),
                    Err(e) => tracing::warn!(error = %e, %peer_id, "dial failed"),
                }
            }
            P2pCommand::Disconnect { peer_id } => {
                tracing::info!(%peer_id, "disconnecting peer");
                if swarm.disconnect_peer_id(peer_id).is_err() {
                    tracing::warn!(%peer_id, "disconnect failed");
                }
            }
            P2pCommand::GetPeerSuiAddress { peer_id, reply } => {
                let mut key = b"zing-sui-addr\0".to_vec();
                key.extend_from_slice(&peer_id.to_bytes());
                let record_key = kad::RecordKey::new(&key);
                let query_id = swarm.behaviour_mut().kad.get_record(record_key);
                pending_sui_addr_queries.insert(query_id, reply);
            }
            P2pCommand::GetPeerVault { peer_id, reply } => {
                let mut key = b"zing-vault\0".to_vec();
                key.extend_from_slice(&peer_id.to_bytes());
                let record_key = kad::RecordKey::new(&key);
                let query_id = swarm.behaviour_mut().kad.get_record(record_key);
                pending_vault_queries.insert(query_id, reply);
            }
        }
    }

    async fn handle_swarm_event(
        swarm: &mut Swarm<ZingBehaviour>,
        event: SwarmEvent<ZingBehaviourEvent>,
        pending_finds: &mut HashMap<kad::QueryId, oneshot::Sender<Vec<PeerId>>>,
        pending_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        pending_range_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        pending_sliver_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        pending_addr_queries: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<Vec<Multiaddr>>>,
        peer_addresses: &mut HashMap<PeerId, Vec<Multiaddr>>,
        bootstrap_peers: &mut std::collections::HashSet<PeerId>,
        bootstrap_done: &mut bool,
        pending_announces: &mut Vec<[u8; 32]>,
        announce_retried: &mut std::collections::HashSet<[u8; 32]>,
        pending_sui_addr_queries: &mut HashMap<kad::QueryId, oneshot::Sender<Option<[u8; 32]>>>,
        pending_vault_queries: &mut HashMap<kad::QueryId, oneshot::Sender<Option<[u8; 32]>>>,
        sui_addr_to_publish: Option<[u8; 32]>,
        vault_id_to_publish: Option<[u8; 32]>,
        sui_addr_published: &mut bool,
        vault_published: &mut bool,
        store: &BlobStoreHandle,
    ) {
        match event {
            SwarmEvent::Behaviour(behaviour_event) => {
                Self::handle_behaviour_event(
                    swarm,
                    behaviour_event,
                    pending_finds,
                    pending_fetches,
                    pending_range_fetches,
                    pending_sliver_fetches,
                    pending_addr_queries,
                    peer_addresses,
                    announce_retried,
                    pending_sui_addr_queries,
                    pending_vault_queries,
                    store,
                )
                .await;
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!(%address, "P2P listening");
            }
            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                tracing::info!(%peer_id, "P2P connection established");
                // Add peer's address to Kad routing table so inbound peers are
                // routable. Without this, provider records with empty addresses
                // (the default for `start_providing`) cannot be resolved via
                // the routing table fallback in `provider_peers()`.
                let peer_addr = match &endpoint {
                    ConnectedPoint::Dialer { address, .. } => address.clone(),
                    ConnectedPoint::Listener { send_back_addr, .. } => send_back_addr.clone(),
                };
                swarm.behaviour_mut().kad.add_address(&peer_id, peer_addr.clone());
                tracing::debug!(%peer_id, addr = %peer_addr, "Kad routing table: added peer address");

                // Publish Sui address on first connection so peers can query it
                if !*sui_addr_published {
                    if let Some(sui_addr) = sui_addr_to_publish {
                        publish_sui_addr_record(swarm, sui_addr);
                        *sui_addr_published = true;
                    }
                }
                // Publish Vault ID on first connection
                if !*vault_published {
                    if let Some(vault_id) = vault_id_to_publish {
                        publish_vault_record(swarm, vault_id);
                        *vault_published = true;
                    }
                }

                if bootstrap_peers.contains(&peer_id) {
                    if !*bootstrap_done {
                        match swarm.behaviour_mut().kad.bootstrap() {
                            Ok(_) => {
                                tracing::info!("kad bootstrap initiated after connection to bootstrap peer {}", peer_id);
                                *bootstrap_done = true;
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "kad bootstrap failed after connection to bootstrap peer {}", peer_id);
                            }
                        }
                    } else {
                        // Reconnected to bootstrap peer — re-announce all cached blobs
                        // in case Fly restarted and lost MemoryStore provider records.
                        if let Ok(ids) = store.read().await.list_blob_ids() {
                            for id_str in &ids {
                                if let Ok(blob_id) = id_str.parse::<BlobId>() {
                                    let key = kad::RecordKey::new(&blob_id.0);
                                    if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
                                        tracing::warn!(blob_id = %id_str, error = %e, "Kad start_providing re-announce failed");
                                    } else {
                                        tracing::info!(blob_id = %id_str, "Kad start_providing re-announced on reconnect");
                                    }
                                }
                            }
                        }
                    }
                }
                if !pending_announces.is_empty() {
                    let announces: Vec<[u8; 32]> = pending_announces.drain(..).collect();
                    for bid in announces {
                        let key = kad::RecordKey::new(&bid);
                        if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
                            tracing::warn!(error = %e, "Kad start_providing retry failed");
                        } else {
                            tracing::info!(blob_id = %hex::encode(bid), "Kad start_providing retry sent");
                        }
                    }
                }
            }
            SwarmEvent::ConnectionClosed {
                peer_id, cause, ..
            } => {
                tracing::info!(peer_id = %peer_id, cause = ?cause, "P2P connection closed");
            }
            SwarmEvent::Dialing { peer_id, .. } => {
                tracing::info!(peer_id = ?peer_id, "P2P dialing peer");
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                tracing::warn!(peer_id = ?peer_id, %error, "P2P outgoing connection failed");
            }
            SwarmEvent::IncomingConnection { .. } => {}
            _ => {}
        }
    }

    async fn handle_behaviour_event(
        swarm: &mut Swarm<ZingBehaviour>,
        event: ZingBehaviourEvent,
        pending_finds: &mut HashMap<kad::QueryId, oneshot::Sender<Vec<PeerId>>>,
        pending_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        pending_range_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        pending_sliver_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        pending_addr_queries: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<Vec<Multiaddr>>>,
        peer_addresses: &mut HashMap<PeerId, Vec<Multiaddr>>,
        announce_retried: &mut std::collections::HashSet<[u8; 32]>,
        pending_sui_addr_queries: &mut HashMap<kad::QueryId, oneshot::Sender<Option<[u8; 32]>>>,
        pending_vault_queries: &mut HashMap<kad::QueryId, oneshot::Sender<Option<[u8; 32]>>>,
        store: &BlobStoreHandle,
    ) {
        match event {
            ZingBehaviourEvent::Kad(kad_event) => {
                match kad_event {
                    kad::Event::RoutingUpdated { peer, addresses, .. } => {
                        let addrs: Vec<Multiaddr> = addresses.iter().cloned().collect();
                        if !addrs.is_empty() {
                            peer_addresses.insert(peer, addrs);
                        }
                    }
                    kad::Event::OutboundQueryProgressed { id, result, .. } => {
                        match result {
                            kad::QueryResult::GetProviders(Ok(ok)) => {
                                match ok {
                                    kad::GetProvidersOk::FoundProviders { providers, .. } => {
                                        if let Some(sender) = pending_finds.remove(&id) {
                                            let peers: Vec<PeerId> = providers.into_iter().collect();
                                            let _ = sender.send(peers);
                                        }
                                    }
                                    kad::GetProvidersOk::FinishedWithNoAdditionalRecord { .. } => {
                                        if let Some(sender) = pending_finds.remove(&id) {
                                            let _ = sender.send(vec![]);
                                        }
                                    }
                                }
                            }
                            kad::QueryResult::GetProviders(Err(e)) => {
                                tracing::warn!(?id, %e, "get_providers query failed");
                                if let Some(sender) = pending_finds.remove(&id) {
                                    let _ = sender.send(vec![]);
                                }
                            }
                            kad::QueryResult::StartProviding(Ok(ok)) => {
                                tracing::info!(key = ?ok.key, "Kad start_providing succeeded: provider record published");
                                let key_bytes = ok.key.to_vec();
                                if key_bytes.len() == 32 {
                                    let mut blob_id = [0u8; 32];
                                    blob_id.copy_from_slice(&key_bytes);
                                    if announce_retried.insert(blob_id) {
                                        tracing::info!(blob_id = %hex::encode(blob_id), "Kad start_providing: scheduling immediate retry to cover fire-and-forget ADD_PROVIDER gap");
                                        let key = kad::RecordKey::new(&blob_id);
                                        if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
                                            tracing::warn!(error = %e, "Kad start_providing retry failed");
                                        }
                                    }
                                }
                            }
                            kad::QueryResult::StartProviding(Err(e)) => {
                                tracing::warn!(?id, %e, "Kad start_providing query failed");
                            }
                            kad::QueryResult::Bootstrap(Ok(ok)) => {
                                tracing::info!(?id, remaining = ok.num_remaining, "Kad bootstrap progress");
                            }
                            kad::QueryResult::Bootstrap(Err(e)) => {
                                tracing::warn!(?id, %e, "Kad bootstrap query failed");
                            }
                            kad::QueryResult::GetRecord(Ok(ok)) => {
                                let result = get_record_value(ok);
                                if let Some(sender) = pending_sui_addr_queries.remove(&id) {
                                    let _ = sender.send(result);
                                }
                                if let Some(sender) = pending_vault_queries.remove(&id) {
                                    let _ = sender.send(result);
                                }
                            }
                            kad::QueryResult::GetRecord(Err(e)) => {
                                tracing::warn!(?id, %e, "Kad get_record query failed");
                                if let Some(sender) = pending_sui_addr_queries.remove(&id) {
                                    let _ = sender.send(None);
                                }
                                if let Some(sender) = pending_vault_queries.remove(&id) {
                                    let _ = sender.send(None);
                                }
                            }
                            kad::QueryResult::PutRecord(Ok(_)) => {
                                tracing::info!("Sui address record published to Kad DHT");
                            }
                            kad::QueryResult::PutRecord(Err(e)) => {
                                tracing::warn!(%e, "Sui address record publish failed");
                            }
                            _ => {
                                tracing::debug!(?id, result = ?result, "Kad query progressed (unhandled)");
                            }
                        }
                    }
                    kad::Event::InboundRequest { request } => {
                        match request {
                            kad::InboundRequest::AddProvider { record } => {
                                if let Some(rec) = record {
                                    tracing::info!(key = ?rec.key, provider = %rec.provider, "Kad InboundRequest: received AddProvider from remote peer");
                                } else {
                                    tracing::info!("Kad InboundRequest: received AddProvider (record stored)");
                                }
                            }
                            _ => {
                                tracing::trace!(?request, "Kad InboundRequest (unhandled)");
                            }
                        }
                    }
                    _ => {}
                }
            }
            ZingBehaviourEvent::Data(data_event) => {
                match data_event {
                    request_response::Event::Message { message, .. } => {
                        match message {
                            request_response::Message::Request {
                                request, channel, ..
                            } => {
                                let response = handle_inbound_request(store, request).await;
                                if let Err(e) = swarm
                                    .behaviour_mut()
                                    .data
                                    .send_response(channel, response)
                                {
                                    tracing::warn!(error = ?e, "send_response");
                                }
                            }
                            request_response::Message::Response {
                                request_id,
                                response,
                                ..
                            } => {
                                if let Some(sender) = pending_fetches.remove(&request_id) {
                                    let result = if let Some(data) = response.data {
                                        Ok(data)
                                    } else {
                                        Err(ZingError::BlobNotFound(
                                            "peer responded not found".into(),
                                        ))
                                    };
                                    let _ = sender.send(result);
                                }
                            }
                        }
                    }
                    request_response::Event::OutboundFailure {
                        request_id,
                        peer,
                        error,
                    } => {
                        tracing::warn!(%request_id, %peer, %error, "outbound request failed to peer (disconnected?)");
                        if let Some(sender) = pending_fetches.remove(&request_id) {
                            let _ = sender.send(Err(ZingError::P2PNetwork(error.to_string())));
                        }
                    }
                    request_response::Event::InboundFailure { error, .. } => {
                        tracing::warn!(%error, "inbound request failed");
                    }
                    _ => {}
                }
            }
            ZingBehaviourEvent::Range(range_event) => {
                match range_event {
                    request_response::Event::Message { message, .. } => {
                        match message {
                            request_response::Message::Request {
                                request, channel, ..
                            } => {
                                let response = handle_inbound_range_request(store, request).await;
                                if let Err(e) = swarm
                                    .behaviour_mut()
                                    .range
                                    .send_response(channel, response)
                                {
                                    tracing::warn!(error = ?e, "send_response");
                                }
                            }
                            request_response::Message::Response {
                                request_id,
                                response,
                                ..
                            } => {
                                if let Some(sender) = pending_range_fetches.remove(&request_id) {
                                    let result = if let Some(data) = response.data {
                                        Ok(data)
                                    } else {
                                        Err(ZingError::BlobNotFound(
                                            "peer responded not found".into(),
                                        ))
                                    };
                                    let _ = sender.send(result);
                                }
                            }
                        }
                    }
                    request_response::Event::OutboundFailure {
                        request_id,
                        peer,
                        error,
                    } => {
                        tracing::warn!(%request_id, %peer, %error, "range outbound request failed to peer (disconnected?)");
                        if let Some(sender) = pending_range_fetches.remove(&request_id) {
                            let _ = sender.send(Err(ZingError::P2PNetwork(error.to_string())));
                        }
                    }
                    request_response::Event::InboundFailure { error, .. } => {
                        tracing::warn!(%error, "range inbound request failed");
                    }
                    _ => {}
                }
            }
            ZingBehaviourEvent::Sliver(sliver_event) => {
                match sliver_event {
                    request_response::Event::Message { message, .. } => {
                        match message {
                            request_response::Message::Request {
                                request, channel, ..
                            } => {
                                let response = handle_inbound_sliver_request(store, request).await;
                                if let Err(e) = swarm
                                    .behaviour_mut()
                                    .sliver
                                    .send_response(channel, response)
                                {
                                    tracing::warn!(error = ?e, "send_response");
                                }
                            }
                            request_response::Message::Response {
                                request_id,
                                response,
                                ..
                            } => {
                                if let Some(sender) = pending_sliver_fetches.remove(&request_id) {
                                    let result = if let Some(data) = response.data {
                                        Ok(data)
                                    } else {
                                        Err(ZingError::BlobNotFound(
                                            "peer responded not found".into(),
                                        ))
                                    };
                                    let _ = sender.send(result);
                                }
                            }
                        }
                    }
                    request_response::Event::OutboundFailure {
                        request_id,
                        peer,
                        error,
                    } => {
                        tracing::warn!(%request_id, %peer, %error, "sliver outbound request failed to peer (disconnected?)");
                        if let Some(sender) = pending_sliver_fetches.remove(&request_id) {
                            let _ = sender.send(Err(ZingError::P2PNetwork(error.to_string())));
                        }
                    }
                    request_response::Event::InboundFailure { error, .. } => {
                        tracing::warn!(%error, "sliver inbound request failed");
                    }
                    _ => {}
                }
            }
            ZingBehaviourEvent::Addr(addr_event) => {
                match addr_event {
                    request_response::Event::Message { message, .. } => {
                        match message {
                            request_response::Message::Request {
                                request, channel, ..
                            } => {
                                let addrs = peer_addresses.get(&request.peer_id).cloned().unwrap_or_default();
                                let response = AddrResponse::found(addrs);
                                if let Err(e) = swarm
                                    .behaviour_mut()
                                    .addr
                                    .send_response(channel, response)
                                {
                                    tracing::warn!(error = ?e, "addr send_response");
                                }
                            }
                            request_response::Message::Response {
                                request_id,
                                response,
                                ..
                            } => {
                                if let Some(sender) = pending_addr_queries.remove(&request_id) {
                                    let _ = sender.send(response.addresses);
                                }
                            }
                        }
                    }
                    request_response::Event::OutboundFailure {
                        request_id,
                        peer,
                        error,
                    } => {
                        tracing::warn!(%request_id, %peer, %error, "addr query failed");
                        if let Some(sender) = pending_addr_queries.remove(&request_id) {
                            let _ = sender.send(vec![]);
                        }
                    }
                    request_response::Event::InboundFailure { error, .. } => {
                        tracing::warn!(%error, "addr inbound request failed");
                    }
                    _ => {}
                }
            }
            ZingBehaviourEvent::Identify(identify_event) => {
                match identify_event {
                    libp2p::identify::Event::Received { peer_id, info, .. } => {
                        if !info.listen_addrs.is_empty() {
                            // Add the peer's actual listen addresses to the Kad routing table.
                            // This gives the Kad DHT the real dialable addresses, superseding
                            // the NAT-mapped send_back_addr (often an ephemeral source port)
                            // that was added via ConnectionEstablished.
                            for addr in &info.listen_addrs {
                                swarm.behaviour_mut().kad.add_address(&peer_id, addr.clone());
                                tracing::debug!(%peer_id, %addr, "Kad routing table: added peer listen address from Identify");
                            }
                            peer_addresses.insert(peer_id, info.listen_addrs.clone());
                        }
                    }
                    _ => {}
                }
            }
            ZingBehaviourEvent::Ping(ping_event) => {
                tracing::trace!(?ping_event, "ping event");
            }
        }
    }
}

fn publish_sui_addr_record(swarm: &mut Swarm<ZingBehaviour>, sui_address: [u8; 32]) {
    let peer_id = *swarm.local_peer_id();
    let mut key = b"zing-sui-addr\0".to_vec();
    key.extend_from_slice(&peer_id.to_bytes());
    let record = kad::Record::new(kad::RecordKey::new(&key), sui_address.to_vec());
    match swarm.behaviour_mut().kad.put_record(record, libp2p::kad::Quorum::One) {
        Ok(_) => tracing::info!("Publishing Sui address to Kad DHT..."),
        Err(e) => tracing::warn!(error = %e, "Failed to start Sui address publish"),
    }
}

fn publish_vault_record(swarm: &mut Swarm<ZingBehaviour>, vault_id: [u8; 32]) {
    let peer_id = *swarm.local_peer_id();
    let mut key = b"zing-vault\0".to_vec();
    key.extend_from_slice(&peer_id.to_bytes());
    let record = kad::Record::new(kad::RecordKey::new(&key), vault_id.to_vec());
    match swarm.behaviour_mut().kad.put_record(record, libp2p::kad::Quorum::One) {
        Ok(_) => tracing::info!("Publishing Vault ID to Kad DHT..."),
        Err(e) => tracing::warn!(error = %e, "Failed to start Vault ID publish"),
    }
}

fn get_record_value(ok: kad::GetRecordOk) -> Option<[u8; 32]> {
    match ok {
        kad::GetRecordOk::FoundRecord(peer_record) => {
            let val = &peer_record.record.value;
            if val.len() == 32 {
                let mut result = [0u8; 32];
                result.copy_from_slice(val);
                Some(result)
            } else {
                tracing::warn!(len = val.len(), "Kad record has invalid length (expected 32)");
                None
            }
        }
        kad::GetRecordOk::FinishedWithNoAdditionalRecord { .. } => None,
    }
}
