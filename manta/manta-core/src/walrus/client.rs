use walrus_core::encoding::ConsistencyCheckType;
use walrus_core::encoding::EncodingConfig;
use walrus_core::encoding::Primary;
use walrus_core::BlobId;
use walrus_sdk::node_client::WalrusNodeClient;
use walrus_sui::client::SuiReadClient;
use walrus_storage_node_client::api::BlobStatus;

use crate::types::{ZingError, ZingResult};

pub struct WalrusL3Client {
    inner: WalrusNodeClient<SuiReadClient>,
}

impl WalrusL3Client {
    pub fn new(client: WalrusNodeClient<SuiReadClient>) -> Self {
        Self { inner: client }
    }

    pub fn encoding_config(&self) -> &EncodingConfig {
        self.inner.encoding_config()
    }

    pub fn sui_client(&self) -> &SuiReadClient {
        self.inner.sui_client()
    }

    pub async fn read_blob(&self, blob_id: &BlobId) -> ZingResult<Vec<u8>> {
        self.inner
            .read_blob_retry_committees::<Primary>(blob_id, ConsistencyCheckType::Strict)
            .await
            .map_err(|e| ZingError::WalrusClient(e.to_string()))
    }

    pub async fn fetch_metadata(
        &self,
        blob_id: &BlobId,
    ) -> ZingResult<walrus_core::metadata::VerifiedBlobMetadataWithId> {
        let status = self
            .inner
            .get_blob_status_with_retries(blob_id, self.inner.sui_client())
            .await
            .map_err(|e| ZingError::WalrusClient(e.to_string()))?;

        let certified_epoch = status
            .initial_certified_epoch()
            .ok_or_else(|| ZingError::WalrusClient("blob has no certified epoch".into()))?;

        self.inner
            .retrieve_metadata(certified_epoch, blob_id)
            .await
            .map_err(|e| ZingError::WalrusClient(e.to_string()))
    }

    pub async fn check_blob_status(&self, blob_id: &BlobId) -> ZingResult<BlobStatus> {
        self.inner
            .get_blob_status_with_retries(blob_id, self.inner.sui_client())
            .await
            .map_err(|e| ZingError::WalrusClient(e.to_string()))
    }
}