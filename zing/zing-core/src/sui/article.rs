use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Article {
    pub id: String,
    pub owner: String,
    pub deleted: bool,
    pub created_at: u64,
    pub blobs: Vec<BlobRef>,
    pub files: Vec<FileRef>,
    #[serde(default)]
    pub subscription_level: Option<u8>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlobRef {
    pub blob_id: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileRef {
    pub name: String,
    pub blob_id: String,
}