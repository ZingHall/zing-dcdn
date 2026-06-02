use libp2p::kad;
use libp2p::kad::store::MemoryStore;
use libp2p::PeerId;
use libp2p::Multiaddr;

use crate::types::ZingResult;

pub fn add_bootstrap_peer(
    kad: &mut kad::Behaviour<MemoryStore>,
    peer_id: PeerId,
    addr: Multiaddr,
) {
    kad.add_address(&peer_id, addr);
}

pub fn start_providing(
    kad: &mut kad::Behaviour<MemoryStore>,
    blob_id: [u8; 32],
) -> ZingResult<()> {
    let key = kad::RecordKey::new(&blob_id);
    kad.start_providing(key)
        .map_err(|e| crate::types::ZingError::P2PNetwork(e.to_string()))?;
    Ok(())
}

pub fn get_providers(
    kad: &mut kad::Behaviour<MemoryStore>,
    blob_id: [u8; 32],
) -> kad::QueryId {
    let key = kad::RecordKey::new(&blob_id);
    kad.get_providers(key)
}

pub fn bootstrap(
    kad: &mut kad::Behaviour<MemoryStore>,
) -> ZingResult<()> {
    kad.bootstrap()
        .map_err(|e| crate::types::ZingError::P2PNetwork(e.to_string()))?;
    Ok(())
}
