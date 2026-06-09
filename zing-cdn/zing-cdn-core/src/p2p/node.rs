use std::collections::HashMap;
use std::time::Duration;
use futures::stream::StreamExt;
use libp2p::identity;
use libp2p::kad;
use libp2p::request_response;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::p2p::behaviour::ZingBehaviour;
use crate::p2p::behaviour::ZingBehaviourEvent;
use crate::p2p::handler::{handle_inbound_request, BlobStoreHandle};
use crate::p2p::protocol::BlobRequest;
use crate::types::{ZingError, ZingResult};

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
        reply: oneshot::Sender<ZingResult<Vec<u8>>>,
    },
    AddBootstrapPeer {
        peer_id: PeerId,
        addr: Multiaddr,
    },
    Bootstrap,
    GetConnectedPeers { reply: oneshot::Sender<Vec<PeerId>> },
    Dial { peer_id: PeerId, addr: Multiaddr },
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
    ) -> ZingResult<()> {

        let store_for_builder = store.clone();
        let mut swarm = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_quic()
            .with_behaviour(move |key| ZingBehaviour::new(key, store_for_builder))
            .map_err(|e| ZingError::P2PNetwork(e.to_string()))?
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(30)))
            .build();

        swarm
            .listen_on(listen_addr)
            .map_err(|e| ZingError::P2PNetwork(e.to_string()))?;

        for (peer_id, addr) in &bootstrap_addrs {
            swarm.behaviour_mut().kad.add_address(peer_id, addr.clone());
        }

        if let Err(e) = swarm.behaviour_mut().kad.bootstrap() {
            tracing::warn!(error = %e, "kad bootstrap");
        }

        let mut pending_finds: HashMap<kad::QueryId, oneshot::Sender<Vec<PeerId>>> = HashMap::new();
        let mut pending_fetches: HashMap<
            request_response::OutboundRequestId,
            oneshot::Sender<ZingResult<Vec<u8>>>,
        > = HashMap::new();

        loop {
            tokio::select! {
                Some(cmd) = command_rx.recv() => {
                    Self::handle_command(
                        &mut swarm,
                        cmd,
                        &mut pending_finds,
                        &mut pending_fetches,
                    );
                }
                event = swarm.next() => {
                    match event {
                        Some(event) => Self::handle_swarm_event(
                            &mut swarm,
                            event,
                            &mut pending_finds,
                            &mut pending_fetches,
                            &store,
                        ).await,
                        None => break,
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
    ) {
        match cmd {
            P2pCommand::AnnounceBlob { blob_id } => {
                let key = kad::RecordKey::new(&blob_id);
                if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
                    tracing::warn!(error = %e, "start_providing");
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
                reply,
            } => {
                let request = BlobRequest {
                    blob_id,
                    version: 0,
                };
                let request_id = swarm
                    .behaviour_mut()
                    .data
                    .send_request(&peer_id, request);
                pending_fetches.insert(request_id, reply);
            }
            P2pCommand::AddBootstrapPeer { peer_id, addr } => {
                swarm.behaviour_mut().kad.add_address(&peer_id, addr);
            }
            P2pCommand::Bootstrap => {
                if let Err(e) = swarm.behaviour_mut().kad.bootstrap() {
                    tracing::warn!(error = %e, "kad bootstrap");
                }
            }
            P2pCommand::GetConnectedPeers { reply } => {
                let peers: Vec<PeerId> = swarm.connected_peers().copied().collect();
                let _ = reply.send(peers);
            }
            P2pCommand::Dial { peer_id, addr } => {
                let mut dial_addr = addr.clone();
                dial_addr.push(libp2p::multiaddr::Protocol::P2p(peer_id));
                eprintln!("P2P dialing {peer_id} at {dial_addr}");
                match swarm.dial(dial_addr) {
                    Ok(()) => tracing::info!(%peer_id, "dialing peer"),
                    Err(e) => {
                        tracing::warn!(error = %e, %peer_id, "dial failed");
                        eprintln!("P2P dial failed: {e}");
                    }
                }
            }
        }
    }

    async fn handle_swarm_event(
        swarm: &mut Swarm<ZingBehaviour>,
        event: SwarmEvent<ZingBehaviourEvent>,
        pending_finds: &mut HashMap<kad::QueryId, oneshot::Sender<Vec<PeerId>>>,
        pending_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
        store: &BlobStoreHandle,
    ) {
        match event {
            SwarmEvent::Behaviour(behaviour_event) => {
                Self::handle_behaviour_event(
                    swarm,
                    behaviour_event,
                    pending_finds,
                    pending_fetches,
                    store,
                )
                .await;
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!(%address, "P2P listening");
                eprintln!("P2P listening on {address}");
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                tracing::info!(%peer_id, "P2P connection established");
                eprintln!("P2P connected to {peer_id}");
            }
            SwarmEvent::ConnectionClosed {
                peer_id, cause, ..
            } => {
                tracing::info!(peer_id = %peer_id, cause = ?cause, "P2P connection closed");
                eprintln!("P2P disconnected from {peer_id}: {cause:?}");
            }
            SwarmEvent::Dialing { peer_id, .. } => {
                tracing::info!(peer_id = ?peer_id, "P2P dialing peer");
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                tracing::warn!(peer_id = ?peer_id, %error, "P2P outgoing connection failed");
                eprintln!("P2P dial error to {peer_id:?}: {error}");
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
        store: &BlobStoreHandle,
    ) {
        match event {
            ZingBehaviourEvent::Kad(kad_event) => {
                if let kad::Event::OutboundQueryProgressed { id, result, .. } = kad_event {
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
                        _ => {}
                    }
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
                        error,
                        ..
                    } => {
                        tracing::warn!(%request_id, %error, "outbound request failed");
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
            ZingBehaviourEvent::Identify(identify_event) => {
                tracing::debug!(?identify_event, "identify event");
            }
            ZingBehaviourEvent::Ping(ping_event) => {
                tracing::trace!(?ping_event, "ping event");
            }
        }
    }
}
