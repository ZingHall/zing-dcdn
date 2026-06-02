use crate::cache::store::BlobStore;
use crate::p2p::protocol::BlobRequest;
use crate::p2p::protocol::BlobResponse;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type BlobStoreHandle = Arc<RwLock<BlobStore>>;

pub async fn handle_inbound_request(
    store: &BlobStoreHandle,
    request: BlobRequest,
) -> BlobResponse {
    let blob_id_hex = hex::encode(request.blob_id);
    tracing::info!(blob_id = %blob_id_hex, "handling inbound blob request");

    let store = store.read().await;
    match store.get(&blob_id_hex) {
        Ok(Some(data)) => {
            tracing::info!(blob_id = %blob_id_hex, size = data.len(), "responding HAVE");
            BlobResponse::have(data)
        }
        Ok(None) => {
            tracing::info!(blob_id = %blob_id_hex, "responding NOT_FOUND");
            BlobResponse::not_found()
        }
        Err(e) => {
            tracing::error!(blob_id = %blob_id_hex, error = %e, "store read error");
            BlobResponse::not_found()
        }
    }
}
