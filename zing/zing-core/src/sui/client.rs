// ZingError will be used when SuiClient methods are implemented
// during integration with the Walrus SuiReadClient.

/// SuiClient wraps the walrus-sui SuiReadClient for on-chain data access.
///
/// In the MVP, this provides:
/// - Epoch/committee info (for checking blob expiry)
/// - Article object reads (for blob metadata lookup)
///
/// The SuiReadClient is created by the WalrusNodeClient and can be
/// accessed via `WalrusNodeClient::sui_client()`.
pub struct SuiClient {
    // The SuiReadClient comes from the WalrusNodeClient.
    // We'll hold a reference to it or create it alongside.
    // For now, we use the walrus-sui types directly.
    inner: std::marker::PhantomData<()>,
}

impl SuiClient {
    pub fn new() -> Self {
        Self {
            inner: std::marker::PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EpochInfo {
    pub current_epoch: u64,
    pub is_active: bool,
    pub epoch_end: Option<chrono::DateTime<chrono::Utc>>,
}