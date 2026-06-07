use std::sync::Arc;

use walrus_sdk::ObjectID;
use walrus_sui::client::SuiReadClient;

/// Thin wrapper around a shared SuiReadClient for on-chain data access.
///
/// Delegates to walrus_sui's read client for:
/// - System/staking object IDs
/// - Epoch and committee info
/// - Blob status checks (via higher-level WalrusL3Client)
#[derive(Clone)]
pub struct SuiClient {
    inner: Arc<SuiReadClient>,
}

impl SuiClient {
    pub fn new(inner: Arc<SuiReadClient>) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &SuiReadClient {
        &self.inner
    }

    pub fn system_object_id(&self) -> ObjectID {
        self.inner.system_object_id()
    }

    pub fn staking_object_id(&self) -> ObjectID {
        self.inner.staking_object_id()
    }
}

#[derive(Debug, Clone)]
pub struct EpochInfo {
    pub current_epoch: u64,
    pub is_active: bool,
    pub epoch_end: Option<chrono::DateTime<chrono::Utc>>,
}
