use crate::cache::store::BlobStore;
use crate::p2p::protocol::BlobRequest;
use crate::p2p::protocol::BlobResponse;
use crate::p2p::protocol::RangeRequest;
use crate::p2p::protocol::RangeResponse;
use crate::p2p::protocol::SliverRequest;
use crate::p2p::protocol::SliverResponse;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use walrus_core::BlobId;

pub type BlobStoreHandle = Arc<RwLock<BlobStore>>;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn is_spot_check() -> bool {
    REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed).is_multiple_of(10)
}

pub async fn handle_inbound_request(
    store: &BlobStoreHandle,
    request: BlobRequest,
) -> BlobResponse {
    let blob_id_str = BlobId(request.blob_id).to_string();
    if request.payment_tx_digest == [0u8; 32] {
        tracing::debug!(blob_id = %blob_id_str, "handling inbound blob request (no payment)");
        if is_spot_check() {
            tracing::warn!(blob_id = %blob_id_str, "Spot-check: refusing unpaid blob request");
            return BlobResponse::not_found();
        }
    } else {
        tracing::debug!(blob_id = %blob_id_str, tx_digest = %hex::encode(request.payment_tx_digest), "handling inbound blob request (paid)");
    }

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
    if request.payment_tx_digest == [0u8; 32] {
        tracing::debug!(blob_id = %blob_id_str, "handling inbound range request (no payment)");
        if is_spot_check() {
            tracing::warn!(blob_id = %blob_id_str, "Spot-check: refusing unpaid range request");
            return RangeResponse::not_found();
        }
    } else {
        tracing::debug!(blob_id = %blob_id_str, tx_digest = %hex::encode(request.payment_tx_digest), "handling inbound range request (paid)");
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spot_check_every_10th() {
        let mut checks = Vec::new();
        for i in 0..1000 {
            if is_spot_check() {
                checks.push(i + 1);
            }
        }
        assert_eq!(checks.len(), 100, "must have exactly 100 checks in 1000 calls");

        let intervals: Vec<usize> = checks.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(intervals.iter().all(|&d| d == 10),
            "spot checks must be exactly 10 positions apart");
    }
}
