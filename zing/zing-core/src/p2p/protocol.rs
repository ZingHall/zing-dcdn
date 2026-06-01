use serde::{Deserialize, Serialize};

pub const ZING_BLOB_PROTOCOL: &str = "/zing/blob/1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobRequest {
    pub blob_id: [u8; 32],
    pub version: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlobResponse {
    Have { size: u64 },
    NotFound,
}