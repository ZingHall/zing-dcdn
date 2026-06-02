use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use libp2p::Multiaddr;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::p2p::node::ZingP2pNode;
use zing_cdn_core::p2p::P2pCommand;

fn create_store() -> Arc<RwLock<BlobStore>> {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = BlobStore::open(dir.path()).expect("open store");
    Arc::new(RwLock::new(store))
}

#[tokio::test]
async fn test_p2p_node_starts_and_listens() {
    let store = create_store();
    let (node, cmd_rx) = ZingP2pNode::new(store.clone());
    let key = node.key().clone();
    let tx = node.command_tx().clone();

    let listen_addr: Multiaddr = "/ip4/127.0.0.1/udp/0/quic-v1"
        .parse()
        .expect("valid multiaddr");

    let handle = tokio::spawn(async move {
        ZingP2pNode::run(key, cmd_rx, store, listen_addr, vec![]).await
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(!handle.is_finished(), "P2P node should still be running");

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn test_p2p_node_announce_blob() {
    let store = create_store();
    let (node, cmd_rx) = ZingP2pNode::new(store.clone());
    let key = node.key().clone();
    let tx = node.command_tx().clone();

    let listen_addr: Multiaddr = "/ip4/127.0.0.1/udp/0/quic-v1"
        .parse()
        .expect("valid multiaddr");

    let handle = tokio::spawn(async move {
        ZingP2pNode::run(key, cmd_rx, store, listen_addr, vec![]).await
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    let blob_id = [0u8; 32];
    tx.send(P2pCommand::AnnounceBlob { blob_id })
        .await
        .expect("send announce");

    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.abort();
    let _ = handle.await;
}

#[test]
fn test_parse_bootstrap_peer_address() {
    let addr: Result<Multiaddr, _> = "/ip4/127.0.0.1/udp/34291/quic-v1".parse();
    assert!(addr.is_ok(), "valid multiaddr should parse");
}
