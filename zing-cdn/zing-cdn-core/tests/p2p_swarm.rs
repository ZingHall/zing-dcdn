use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use libp2p::Multiaddr;
use tokio::sync::oneshot;

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
    let _tx = node.command_tx().clone();

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

#[tokio::test]
#[ignore]
async fn test_node_to_node_blob_transfer() {
    let store_a = create_store();
    let store_b = create_store();

    let test_data = b"Hello from P2P! This is test blob data.";
    let test_blob_id = "test_blob";
    let blob_id_bytes: [u8; 32] = {
        let mut b = [0u8; 32];
        b[..test_blob_id.len()].copy_from_slice(test_blob_id.as_bytes());
        b
    };

    // Insert blob into Node A's store
    store_a.write().await.put(test_blob_id, test_data).expect("put");

    // Start Node A on fixed port
    let (node_a, rx_a) = ZingP2pNode::new(store_a.clone());
    let key_a = node_a.key().clone();
    let tx_a = node_a.command_tx().clone();
    let peer_a = node_a.local_peer_id();
    let listen_a: Multiaddr = "/ip4/127.0.0.1/udp/19001/quic-v1".parse().expect("addr");
    let listen_a_for_b = listen_a.clone();
    let store_a_clone = store_a.clone();
    let join_a = tokio::spawn(async move {
        let _ = ZingP2pNode::run(key_a, rx_a, store_a_clone, listen_a, vec![]).await;
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Node A announces the blob
    tx_a.send(P2pCommand::AnnounceBlob { blob_id: blob_id_bytes }).await.expect("announce");

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Start Node B (no bootstrap peers — add them after startup)
    let (node_b, rx_b) = ZingP2pNode::new(store_b.clone());
    let key_b = node_b.key().clone();
    let tx_b = node_b.command_tx().clone();
    let listen_b: Multiaddr = "/ip4/127.0.0.1/udp/19002/quic-v1".parse().expect("addr");
    let store_b_clone = store_b.clone();
    let join_b = tokio::spawn(async move {
        let _ = ZingP2pNode::run(key_b, rx_b, store_b_clone, listen_b.clone(), vec![]).await;
    });

    // Let Node B start, then directly dial Node A
    tokio::time::sleep(Duration::from_secs(1)).await;
    tx_b.send(P2pCommand::Dial {
        peer_id: peer_a,
        addr: listen_a_for_b.clone(),
    }).await.expect("dial");

    // Wait for Node B to connect to Node A (poll with retries)
    eprintln!("Waiting for Node B to connect to Node A...");
    let connected = loop {
        let (reply, rx) = oneshot::channel();
        tx_b.send(P2pCommand::GetConnectedPeers { reply }).await.expect("get connected");
        if let Some(peers) = tokio::time::timeout(Duration::from_secs(2), rx).await.ok().and_then(|r| r.ok()) {
            if peers.contains(&peer_a) {
                break peers;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    eprintln!("Node B connected to Node A: {connected:?}");

    // Add Node A to Kademlia routing table and bootstrap
    tx_b.send(P2pCommand::AddBootstrapPeer {
        peer_id: peer_a,
        addr: listen_a_for_b.clone(),
    }).await.expect("add bootstrap");
    tx_b.send(P2pCommand::Bootstrap).await.expect("bootstrap");

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Node B queries for providers of the blob

    // Node B queries for providers of the blob
    let (reply, rx) = oneshot::channel();
    tx_b.send(P2pCommand::FindProviders {
        blob_id: blob_id_bytes,
        reply,
    }).await.expect("find providers");

    let providers = tokio::time::timeout(Duration::from_secs(8), rx)
        .await
        .expect("find providers timeout")
        .expect("find providers oneshot");
    eprintln!("Providers: {providers:?}");

    // Node B should find Node A as a provider
    assert!(!providers.is_empty(), "Node B should find at least one provider");
    assert!(
        providers.contains(&peer_a),
        "Node A ({peer_a}) should be in providers: {providers:?}"
    );

    // Cleanup
    join_a.abort();
    join_b.abort();
    let _ = join_a.await;
    let _ = join_b.await;
}
