use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use libp2p::Multiaddr;
use libp2p::identity;
use tokio::sync::oneshot;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::p2p::node::ZingP2pNode;
use zing_cdn_core::p2p::P2pCommand;

fn create_keypair() -> identity::Keypair {
    identity::Keypair::generate_ed25519()
}

fn create_store() -> Arc<RwLock<BlobStore>> {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = BlobStore::open(dir.path()).expect("open store");
    Arc::new(RwLock::new(store))
}

#[tokio::test]
async fn test_kad_start_providing_and_get_providers() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("zing_cdn_core=debug,libp2p_kad=trace"))
        .with_writer(std::io::stderr)
        .try_init();

    // Node A: bootstrap node + announcer
    let store_a = create_store();
    let (node_a, rx_a) = ZingP2pNode::new(store_a.clone(), create_keypair());
    let key_a = node_a.key().clone();
    let tx_a = node_a.command_tx().clone();
    let peer_a = node_a.local_peer_id();
    let listen_a: Multiaddr = "/ip4/127.0.0.1/udp/19101/quic-v1".parse().unwrap();
    let listen_a_for_b = listen_a.clone();

    let store_a_clone = store_a.clone();
    let join_a = tokio::spawn(async move {
        let _ = ZingP2pNode::run(key_a, rx_a, store_a_clone, listen_a, vec![], vec![], None).await;
    });

    // Wait for Node A to start listening
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Node B: connects to A, queries providers
    let store_b = create_store();
    let (node_b, rx_b) = ZingP2pNode::new(store_b.clone(), create_keypair());
    let key_b = node_b.key().clone();
    let tx_b = node_b.command_tx().clone();
    let listen_b: Multiaddr = "/ip4/127.0.0.1/udp/19102/quic-v1".parse().unwrap();

    let store_b_clone = store_b.clone();
    let join_b = tokio::spawn(async move {
        let _ = ZingP2pNode::run(
            key_b,
            rx_b,
            store_b_clone,
            listen_b,
            vec![(peer_a, listen_a_for_b)],
            vec![],
            None,
        ).await;
    });

    // Wait for Node B to connect, bootstrap, and Kad to stabilize
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Node A announces a blob
    let blob_id = [42u8; 32];
    tx_a.send(P2pCommand::AnnounceBlob { blob_id }).await.expect("announce");

    // Wait for provider record to propagate via ADD_PROVIDER
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Node B queries for providers
    let (reply, rx) = oneshot::channel();
    tx_b.send(P2pCommand::FindProviders { blob_id, reply }).await.expect("find providers");

    let providers = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("timeout waiting for find_providers")
        .expect("oneshot channel");

    eprintln!("Providers found: {:?}", providers);
    assert!(
        providers.contains(&peer_a),
        "Node B should find Node A as a provider. Got: {:?}",
        providers
    );

    join_a.abort();
    join_b.abort();
    let _ = join_a.await;
    let _ = join_b.await;
}
