use crate::cache::store::BlobStore;
use crate::p2p::node::BlobResponse;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type BlobStoreHandle = Arc<RwLock<BlobStore>>;

pub struct BlobRequestHandler {
    store: BlobStoreHandle,
}

impl BlobRequestHandler {
    pub fn new(store: BlobStoreHandle) -> Self {
        Self { store }
    }

    pub fn handle_request(&self, request: crate::p2p::node::BlobRequest) -> BlobResponse {
        let blob_id_hex = hex::encode(request.blob_id);
        tracing::info!(blob_id = %blob_id_hex, "received blob request from peer");

        let store = self.store.blocking_read();
        match store.get(&blob_id_hex) {
            Ok(Some(data)) => {
                tracing::info!(blob_id = %blob_id_hex, size = data.len(), "responding HAVE to peer");
                BlobResponse::Have { size: data.len() as u64 }
            }
            Ok(None) => {
                tracing::info!(blob_id = %blob_id_hex, "responding NOT_FOUND to peer");
                BlobResponse::NotFound
            }
            Err(e) => {
                tracing::error!(blob_id = %blob_id_hex, error = %e, "error reading blob from store");
                BlobResponse::NotFound
            }
        }
    }
}