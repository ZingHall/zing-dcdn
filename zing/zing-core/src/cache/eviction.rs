use crate::cache::pinning::PinningManager;
use crate::cache::store::BlobStore;
use crate::types::{ZingError, ZingResult};

pub struct EvictionManager {
    store: BlobStore,
    budget_bytes: u64,
}

impl EvictionManager {
    pub fn new(store: BlobStore, budget_bytes: u64) -> Self {
        Self { store, budget_bytes }
    }

    pub fn run(&self, pinning: &PinningManager) -> ZingResult<()> {
        let mut total = self.store.total_size()?;
        if total <= self.budget_bytes {
            return Ok(());
        }

        let blob_ids = self.store.list_blob_ids()?;
        let mut candidates: Vec<(String, u64)> = Vec::new();

        for id in &blob_ids {
            if pinning.is_pinned(id)? {
                continue;
            }
            let size = self.store.blob_size(id)?
                .ok_or_else(|| ZingError::Cache(
                    format!("blob {} has no size", id)
                ))?;
            candidates.push((id.clone(), size));
        }

        for (id, size) in candidates {
            if total <= self.budget_bytes {
                break;
            }
            self.store.delete(&id)?;
            total -= size;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::store::BlobStore;
    use crate::cache::pinning::PinningManager;

    #[test]
    fn test_eviction_skips_pinned() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = BlobStore::open(dir.path()).expect("open store");
        let pinning = PinningManager::new(store.clone());
        let eviction = EvictionManager::new(store.clone(), 100);

        let data = b"hello world".to_vec();
        store.put("pinned_blob", &data).expect("put");
        pinning.pin("pinned_blob").expect("pin");

        eviction.run(&pinning).expect("eviction");

        let result = store.get("pinned_blob").expect("get");
        assert!(result.is_some(), "pinned blob should not be evicted");
    }

    #[test]
    fn test_eviction_no_op_if_under_budget() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = BlobStore::open(dir.path()).expect("open store");
        let pinning = PinningManager::new(store.clone());
        let eviction = EvictionManager::new(store.clone(), 10000);

        store.put("blob_a", b"small").expect("put");

        eviction.run(&pinning).expect("eviction");

        let result = store.get("blob_a").expect("get");
        assert!(result.is_some(), "blob should not be evicted when under budget");
    }

    #[test]
    fn test_eviction_removes_oldest_unpinned() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = BlobStore::open(dir.path()).expect("open store");
        let pinning = PinningManager::new(store.clone());
        let eviction = EvictionManager::new(store.clone(), 20);

        let data = b"0123456789".to_vec(); // 10 bytes each
        store.put("blob_a", &data).expect("put a");
        store.put("blob_b", &data).expect("put b");
        // Total: 20 bytes, at budget limit

        let blob_c_data = b"0123456789".to_vec(); // 10 bytes
        store.put("blob_c", &blob_c_data).expect("put c");
        // Total: 30 bytes, over budget

        eviction.run(&pinning).expect("eviction");

        // blob_a (oldest unpinned) should be evicted
        assert!(store.get("blob_a").expect("get").is_none());
    }
}