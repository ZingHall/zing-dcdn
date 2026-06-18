use libp2p::identify;
use libp2p::kad;
use libp2p::kad::store::MemoryStore;
use libp2p::ping;
use libp2p::request_response;
use libp2p::swarm::NetworkBehaviour;
use libp2p::StreamProtocol;
use std::time::Duration;

use crate::p2p::handler::BlobStoreHandle;
use crate::p2p::protocol::BinaryProtocolCodec;
use crate::p2p::protocol::RangeProtocolCodec;
use crate::p2p::protocol::SliverProtocolCodec;
use crate::p2p::protocol::AddrProtocolCodec;

#[derive(NetworkBehaviour)]
pub struct ZingBehaviour {
    pub kad: kad::Behaviour<MemoryStore>,
    pub data: request_response::Behaviour<BinaryProtocolCodec>,
    pub range: request_response::Behaviour<RangeProtocolCodec>,
    pub sliver: request_response::Behaviour<SliverProtocolCodec>,
    pub addr: request_response::Behaviour<AddrProtocolCodec>,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
}

impl ZingBehaviour {
    pub fn new(key: &libp2p::identity::Keypair, _store: BlobStoreHandle) -> Self {
        let peer_id = key.public().to_peer_id();

        let kad_cfg = kad::Config::new(StreamProtocol::new("/zing-cdn/kad/1.0.0"));
        let kad_store = MemoryStore::new(peer_id);
        let kad = kad::Behaviour::with_config(peer_id, kad_store, kad_cfg);

        let data_cfg = request_response::Config::default()
            .with_request_timeout(Duration::from_secs(30));
        let data = request_response::Behaviour::new(
            vec![("/zing-cdn/data/2.0", request_response::ProtocolSupport::Full)],
            data_cfg,
        );

        let range_cfg = request_response::Config::default()
            .with_request_timeout(Duration::from_secs(30));
        let range = request_response::Behaviour::new(
            vec![("/zing-cdn/range/1.0", request_response::ProtocolSupport::Full)],
            range_cfg,
        );

        let sliver_cfg = request_response::Config::default()
            .with_request_timeout(Duration::from_secs(30));
        let sliver = request_response::Behaviour::new(
            vec![("/zing-cdn/sliver/1.0", request_response::ProtocolSupport::Full)],
            sliver_cfg,
        );

        let addr_cfg = request_response::Config::default()
            .with_request_timeout(Duration::from_secs(10));
        let addr = request_response::Behaviour::new(
            vec![("/zing-cdn/addr/1.0", request_response::ProtocolSupport::Full)],
            addr_cfg,
        );

        let identify = identify::Behaviour::new(
            identify::Config::new("zing-cdn/0.1.0".to_string(), key.public())
                .with_interval(Duration::from_secs(45)),
        );

        let ping_cfg = ping::Config::new()
            .with_interval(Duration::from_secs(15));
        let ping = ping::Behaviour::new(ping_cfg);

        Self { kad, data, range, sliver, addr, identify, ping }
    }
}
