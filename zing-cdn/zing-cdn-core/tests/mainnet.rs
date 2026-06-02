use std::sync::Arc;

use tempfile::tempdir;
use tokio::sync::RwLock;
use walrus_core::metadata::BlobMetadataApi;
use walrus_core::BlobId;

use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::client::ZingClient;
use zing_cdn_core::mesh::reputation::PeerReputationTable;
use zing_cdn_core::mesh::resolver::Resolver;
use zing_cdn_core::walrus::verify::BlobVerifier;

const TEST_BLOB_ID: &str = "jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q";

#[tokio::test]
#[ignore]
async fn test_mainnet_connect() {
    let client = ZingClient::from_mainnet().await.expect("connect to mainnet");
    let blob_id: BlobId = TEST_BLOB_ID.parse().expect("parse blob ID");
    let status = client.check_blob_status(&blob_id).await.expect("check blob status");
    let epoch = status.initial_certified_epoch();
    assert!(epoch.is_some(), "blob should have a certified epoch");
    eprintln!("connected to mainnet, certified epoch: {:?}", epoch.unwrap());
}

#[tokio::test]
#[ignore]
async fn test_mainnet_read_blob() {
    let client = ZingClient::from_mainnet().await.expect("connect to mainnet");
    let blob_id: BlobId = TEST_BLOB_ID.parse().expect("parse blob ID");
    let data = client.read_blob(&blob_id).await.expect("read blob from mainnet");
    assert!(!data.is_empty(), "blob data should not be empty");
    eprintln!("read {} bytes from blob {}", data.len(), TEST_BLOB_ID);
}

#[tokio::test]
#[ignore]
async fn test_mainnet_fetch_metadata() {
    let client = ZingClient::from_mainnet().await.expect("connect to mainnet");
    let blob_id: BlobId = TEST_BLOB_ID.parse().expect("parse blob ID");
    let metadata = client
        .fetch_metadata(&blob_id)
        .await
        .expect("fetch metadata");
    assert_eq!(
        metadata.blob_id(),
        &blob_id,
        "metadata blob_id should match requested blob_id"
    );
    let size = metadata.metadata().unencoded_length();
    eprintln!("metadata verified, unencoded length: {} bytes", size);
}

#[tokio::test]
#[ignore]
async fn test_mainnet_verify_blob() {
    let client = ZingClient::from_mainnet().await.expect("connect to mainnet");
    let blob_id: BlobId = TEST_BLOB_ID.parse().expect("parse blob ID");

    let data = client.read_blob(&blob_id).await.expect("read blob");
    let metadata = client
        .fetch_metadata(&blob_id)
        .await
        .expect("fetch metadata");

    let verifier = BlobVerifier::new(client.encoding_config_arc());
    verifier
        .verify_blob_against_metadata(&metadata, &data)
        .expect("blob verification should pass");
    eprintln!(
        "blob {} verified against metadata (size: {})",
        TEST_BLOB_ID,
        data.len()
    );
}

#[tokio::test]
#[ignore]
async fn test_mainnet_resolver() {
    let client = ZingClient::from_mainnet().await.expect("connect to mainnet");
    let blob_id: BlobId = TEST_BLOB_ID.parse().expect("parse blob ID");

    let dir = tempdir().expect("create temp dir");
    let store = BlobStore::open(dir.path()).expect("open blob store");
    let pinning = PinningManager::new(store.clone());
    let eviction = EvictionManager::new(store.clone(), 1024 * 1024 * 100);

    let walrus_client = client.walrus_client_arc();
    let verifier = Arc::new(BlobVerifier::new(client.encoding_config_arc()));

    let resolver = Resolver::new(
        Arc::new(RwLock::new(store)),
        Arc::new(RwLock::new(pinning)),
        Arc::new(RwLock::new(eviction)),
        walrus_client,
        verifier,
        Arc::new(RwLock::new(PeerReputationTable::new())),
    );

    // First resolve: L3 miss → fetch from Walrus
    let result = resolver.resolve(&blob_id).await.expect("resolve blob");
    assert!(!result.data.is_empty(), "resolved data should not be empty");
    eprintln!(
        "first resolve: source={:?}, size={}",
        result.resolution,
        result.data.len()
    );

    // Second resolve: should be L0 cache hit
    let result2 = resolver.resolve(&blob_id).await.expect("resolve blob again");
    assert!(!result2.data.is_empty(), "cached data should not be empty");
    eprintln!(
        "second resolve: source={:?}, cached={}",
        result2.resolution, result2.cached
    );
    assert_eq!(result.data, result2.data, "cached data should match original");
}
