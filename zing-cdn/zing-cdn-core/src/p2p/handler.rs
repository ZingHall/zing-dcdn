use crate::cache::store::BlobStore;
use crate::p2p::protocol::BlobRequest;
use crate::p2p::protocol::BlobResponse;
use crate::p2p::protocol::RangeRequest;
use crate::p2p::protocol::RangeResponse;
use crate::p2p::protocol::SliverRequest;
use crate::p2p::protocol::SliverResponse;
use std::sync::Arc;
use tokio::sync::RwLock;
use walrus_core::BlobId;

pub type BlobStoreHandle = Arc<RwLock<BlobStore>>;

pub async fn handle_inbound_request(
    store: &BlobStoreHandle,
    request: BlobRequest,
) -> BlobResponse {
    let blob_id_str = BlobId(request.blob_id).to_string();
    tracing::info!(blob_id = %blob_id_str, "handling inbound blob request");

    let store = store.read().await;
    match store.get(&blob_id_str) {
        Ok(Some(data)) => {
            tracing::info!(blob_id = %blob_id_str, size = data.len(), "responding HAVE");
            BlobResponse::have(data)
        }
        Ok(None) => {
            tracing::info!(blob_id = %blob_id_str, "responding NOT_FOUND");
            BlobResponse::not_found()
        }
        Err(e) => {
            tracing::error!(blob_id = %blob_id_str, error = %e, "store read error");
            BlobResponse::not_found()
        }
    }
}

pub async fn handle_inbound_range_request(
    store: &BlobStoreHandle,
    request: RangeRequest,
) -> RangeResponse {
    let blob_id_str = BlobId(request.blob_id).to_string();

    let store = store.read().await;
    let data = match store.get(&blob_id_str) {
        Ok(Some(data)) => data,
        Ok(None) => {
            tracing::info!(blob_id = %blob_id_str, "range request: not_found");
            return RangeResponse::not_found();
        }
        Err(e) => {
            tracing::error!(blob_id = %blob_id_str, error = %e, "store read error");
            return RangeResponse::not_found();
        }
    };

    let start = request.offset as usize;
    let end = (start + request.length as usize).min(data.len());
    if start >= data.len() {
        tracing::info!(blob_id = %blob_id_str, offset = start, "range request: out of bounds");
        return RangeResponse::not_found();
    }
    let chunk = data[start..end].to_vec();
    tracing::info!(blob_id = %blob_id_str, offset = start, len = chunk.len(), "range request: served");
    RangeResponse::have(chunk)
}

pub async fn handle_inbound_sliver_request(
    store: &BlobStoreHandle,
    request: SliverRequest,
) -> SliverResponse {
    let blob_id_str = BlobId(request.blob_id).to_string();
    let axis_str = if request.axis == SliverRequest::AXIS_PRIMARY { "primary" } else { "secondary" };
    tracing::info!(
        blob_id = %blob_id_str,
        sliver_index = request.sliver_pair_index,
        axis = axis_str,
        "sliver request"
    );

    let store = store.read().await;
    if store.get(&blob_id_str).map(|o| o.is_some()).unwrap_or(false) {
        tracing::info!(blob_id = %blob_id_str, "sliver request: blob cached but sliver cache not yet implemented");
    }
    SliverResponse::not_found()
}
