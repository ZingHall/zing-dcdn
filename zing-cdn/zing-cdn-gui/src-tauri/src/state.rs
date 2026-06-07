use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use libp2p::{PeerId, Multiaddr};

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::P2pCommand;
use zing_cdn_core::p2p::handler::BlobStoreHandle;

pub struct AppState {
    pub store: Arc<RwLock<BlobStore>>,
    pub pinning: Arc<RwLock<PinningManager>>,
    pub eviction: Arc<RwLock<EvictionManager>>,
    pub p2p_tx: mpsc::Sender<P2pCommand>,
    pub peer_id: PeerId,
    pub listen_addr: Multiaddr,
    pub p2p_store: BlobStoreHandle,
}
