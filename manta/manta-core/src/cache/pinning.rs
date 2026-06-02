use crate::cache::store::BlobStore;
use crate::types::{ZingError, ZingResult};

pub struct PinningManager {
    store: BlobStore,
}

impl PinningManager {
    pub fn new(store: BlobStore) -> Self {
        Self { store }
    }

    pub fn pin(&self, blob_id: &str) -> ZingResult<()> {
        let cf = self.store.db().cf_handle("pins")
            .ok_or_else(|| ZingError::Cache("pins CF not found".into()))?;
        self.store.db().put_cf(&cf, blob_id.as_bytes(), b"1")
            .map_err(|e| ZingError::Cache(e.to_string()))?;
        Ok(())
    }

    pub fn unpin(&self, blob_id: &str) -> ZingResult<()> {
        let cf = self.store.db().cf_handle("pins")
            .ok_or_else(|| ZingError::Cache("pins CF not found".into()))?;
        self.store.db().delete_cf(&cf, blob_id.as_bytes())
            .map_err(|e| ZingError::Cache(e.to_string()))?;
        Ok(())
    }

    pub fn is_pinned(&self, blob_id: &str) -> ZingResult<bool> {
        let cf = self.store.db().cf_handle("pins")
            .ok_or_else(|| ZingError::Cache("pins CF not found".into()))?;
        let result = self.store.db().get_cf(&cf, blob_id.as_bytes())
            .map_err(|e| ZingError::Cache(e.to_string()))?;
        Ok(result.is_some())
    }

    pub fn list_pinned(&self) -> ZingResult<Vec<String>> {
        let cf = self.store.db().cf_handle("pins")
            .ok_or_else(|| ZingError::Cache("pins CF not found".into()))?;
        let iter = self.store.db().iterator_cf(&cf, rocksdb::IteratorMode::Start);
        let mut ids = Vec::new();
        for item in iter {
            let (key, _) = item.map_err(|e| ZingError::Cache(e.to_string()))?;
            let id = String::from_utf8(key.to_vec())
                .map_err(|e| ZingError::Cache(e.to_string()))?;
            ids.push(id);
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::store::BlobStore;

    fn temp_store() -> (BlobStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = BlobStore::open(dir.path()).expect("open store");
        (store, dir)
    }

    #[test]
    fn test_pin_and_unpin_blob() {
        let (store, _dir) = temp_store();
        let pinning = PinningManager::new(store.clone());
        let data = b"hello world".to_vec();

        store.put("test_blob_123", &data).expect("put blob");
        assert!(!pinning.is_pinned("test_blob_123").expect("check pin"));

        pinning.pin("test_blob_123").expect("pin blob");
        assert!(pinning.is_pinned("test_blob_123").expect("check pin"));

        pinning.unpin("test_blob_123").expect("unpin blob");
        assert!(!pinning.is_pinned("test_blob_123").expect("check pin after unpin"));
    }
}