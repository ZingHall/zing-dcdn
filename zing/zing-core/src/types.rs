use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CacheState {
    Pinned,
    Cached,
    Evicted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlobResolution {
    LocalCache,
    L1Peer,
    L3Walrus,
}

#[derive(Debug, thiserror::Error)]
pub enum ZingError {
    #[error("blob not found: {0}")]
    BlobNotFound(String),
    #[error("blob expired (epoch ended)")]
    BlobExpired,
    #[error("Walrus client error: {0}")]
    WalrusClient(String),
    #[error("Sui client error: {0}")]
    SuiClient(String),
    #[error("P2P network error: {0}")]
    P2PNetwork(String),
    #[error("no peers available for blob: {0}")]
    NoPeersAvailable(String),
    #[error("cache error: {0}")]
    Cache(String),
    #[error("verification failed: computed blob ID {computed} does not match expected {expected}")]
    VerificationFailed { computed: String, expected: String },
    #[error("disk full, cannot cache blob")]
    DiskFull,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type ZingResult<T> = Result<T, ZingError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobInfo {
    pub blob_id: String,
    pub size: u64,
    pub state: CacheState,
    pub resolution: Option<BlobResolution>,
    pub pinned: bool,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerScore {
    pub peer_id: String,
    pub score: i32,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub successful_streams: u32,
    pub failed_verifications: u32,
    pub dropped_connections: u32,
    pub false_claims: u32,
}