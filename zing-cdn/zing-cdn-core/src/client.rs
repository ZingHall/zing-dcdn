use std::sync::Arc;

use libp2p::PeerId;
use tokio::sync::mpsc;
use walrus_core::encoding::EncodingConfig;
use walrus_core::metadata::VerifiedBlobMetadataWithId;
use walrus_core::BlobId;
use walrus_sdk::config::ClientConfig;
use walrus_sdk::node_client::WalrusNodeClient;
use walrus_storage_node_client::api::BlobStatus;
use walrus_sui::client::contract_config::ContractConfig;
use walrus_sui::client::retry_client::RetriableSuiClient;
use walrus_sui::client::SuiReadClient;

use crate::p2p::node::P2pCommand;
use crate::sui::SuiClient;
use crate::types::{ZingError, ZingResult};
use crate::walrus::client::WalrusL3Client;

/// ZingClient is the main entry point for connecting to the Walrus network.
///
/// It manages the full lifecycle of a WalrusNodeClient with SuiReadClient,
/// provides convenience methods for blob reading and verification,
/// and can construct a Resolver with local caching.
pub struct ZingClient {
    walrus_client: Arc<WalrusL3Client>,
    encoding_config: Arc<EncodingConfig>,
    p2p_command_tx: Option<mpsc::Sender<P2pCommand>>,
    p2p_peer_id: Option<PeerId>,
}

impl ZingClient {
    /// Connect to Walrus mainnet using hardcoded configuration.
    pub async fn from_mainnet() -> ZingResult<Self> {
        Self::from_config(mainnet_config()).await
    }

    /// Connect to a Walrus network with the given configuration.
    pub async fn from_config(config: ClientConfig) -> ZingResult<Self> {
        let retriable_client = RetriableSuiClient::new_for_rpc_urls(
            &config.rpc_urls,
            config.backoff_config().clone(),
            None,
        )
        .map_err(|e| ZingError::SuiClient(e.to_string()))?;

        let sui_read_client = config
            .new_read_client(retriable_client)
            .await
            .map_err(|e| ZingError::SuiClient(e.to_string()))?;

        let refresh_handle = config
            .build_refresher_and_run(sui_read_client.clone())
            .await
            .map_err(|e| ZingError::WalrusClient(e.to_string()))?;

        let n_shards = refresh_handle.n_shards();
        let encoding_config = Arc::new(EncodingConfig::new(n_shards));

        let node_client = WalrusNodeClient::new_read_client(config, refresh_handle, sui_read_client)
            .map_err(|e| ZingError::WalrusClient(e.to_string()))?;

        let walrus_client = Arc::new(WalrusL3Client::new(node_client));

        Ok(Self {
            walrus_client,
            encoding_config,
            p2p_command_tx: None,
            p2p_peer_id: None,
        })
    }

    pub fn walrus_client(&self) -> &WalrusL3Client {
        &self.walrus_client
    }

    pub fn walrus_client_arc(&self) -> Arc<WalrusL3Client> {
        self.walrus_client.clone()
    }

    pub fn encoding_config(&self) -> &EncodingConfig {
        &self.encoding_config
    }

    pub fn encoding_config_arc(&self) -> Arc<EncodingConfig> {
        self.encoding_config.clone()
    }

    pub fn sui_read_client(&self) -> &SuiReadClient {
        self.walrus_client.sui_client()
    }

    /// Returns a SuiClient wrapper sharing the underlying SuiReadClient.
    pub fn sui_client(&self) -> SuiClient {
        SuiClient::new(Arc::new(self.sui_read_client().clone()))
    }

    pub async fn read_blob(&self, blob_id: &BlobId) -> ZingResult<Vec<u8>> {
        self.walrus_client.read_blob(blob_id).await
    }

    pub async fn fetch_metadata(&self, blob_id: &BlobId) -> ZingResult<VerifiedBlobMetadataWithId> {
        self.walrus_client.fetch_metadata(blob_id).await
    }

    pub async fn check_blob_status(&self, blob_id: &BlobId) -> ZingResult<BlobStatus> {
        self.walrus_client.check_blob_status(blob_id).await
    }

    pub fn set_p2p_handle(
        &mut self,
        command_tx: mpsc::Sender<P2pCommand>,
        peer_id: PeerId,
    ) {
        self.p2p_command_tx = Some(command_tx);
        self.p2p_peer_id = Some(peer_id);
    }

    pub fn p2p_command_tx(&self) -> Option<&mpsc::Sender<P2pCommand>> {
        self.p2p_command_tx.as_ref()
    }

    pub fn p2p_peer_id(&self) -> Option<&PeerId> {
        self.p2p_peer_id.as_ref()
    }
}

fn mainnet_config() -> ClientConfig {
    let system_object = walrus_sdk::ObjectID::from_hex_literal(
        "0x2134d52768ea07e8c43570ef975eb3e4c27a39fa6396bef985b5abc58d03ddd2",
    )
    .expect("valid mainnet system object");

    let staking_object = walrus_sdk::ObjectID::from_hex_literal(
        "0x10b9d30c28448939ce6c4d6c6e0ffce4a7f8a4ada8248bdad09ef8b70e4a3904",
    )
    .expect("valid mainnet staking object");

    let contract_config = ContractConfig::new(system_object, staking_object);
    let mut config = ClientConfig::new_from_contract_config(contract_config);
    config
        .rpc_urls
        .push("https://fullnode.mainnet.sui.io:443".to_string());
    config
}
