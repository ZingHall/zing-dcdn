use rocksdb::{DB, Options, ColumnFamilyDescriptor};
use std::path::Path;
use std::sync::Arc;
use crate::types::{ZingError, ZingResult};

const BLOBS_CF: &str = "blobs";
const METADATA_CF: &str = "metadata";
const PINS_CF: &str = "pins";

#[derive(Clone)]
pub struct BlobStore {
    db: Arc<DB>,
}

impl BlobStore {
    pub fn open(path: &Path) -> ZingResult<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = vec![
            ColumnFamilyDescriptor::new(BLOBS_CF, Options::default()),
            ColumnFamilyDescriptor::new(METADATA_CF, Options::default()),
            ColumnFamilyDescriptor::new(PINS_CF, Options::default()),
        ];

        let db = DB::open_cf_descriptors(&opts, path, cfs)
            .map_err(|e| ZingError::Cache(e.to_string()))?;
        Ok(Self { db: Arc::new(db) })
    }

    pub fn put(&self, blob_id: &str, data: &[u8]) -> ZingResult<()> {
        let cf = self.db.cf_handle(BLOBS_CF)
            .ok_or_else(|| ZingError::Cache("blobs CF not found".into()))?;
        self.db.put_cf(&cf, blob_id.as_bytes(), data)
            .map_err(|e| ZingError::Cache(e.to_string()))?;
        Ok(())
    }

    pub fn get(&self, blob_id: &str) -> ZingResult<Option<Vec<u8>>> {
        let cf = self.db.cf_handle(BLOBS_CF)
            .ok_or_else(|| ZingError::Cache("blobs CF not found".into()))?;
        let result = self.db.get_cf(&cf, blob_id.as_bytes())
            .map_err(|e| ZingError::Cache(e.to_string()))?;
        Ok(result)
    }

    pub fn delete(&self, blob_id: &str) -> ZingResult<()> {
        let cf = self.db.cf_handle(BLOBS_CF)
            .ok_or_else(|| ZingError::Cache("blobs CF not found".into()))?;
        self.db.delete_cf(&cf, blob_id.as_bytes())
            .map_err(|e| ZingError::Cache(e.to_string()))?;
        Ok(())
    }

    pub fn put_metadata(&self, blob_id: &str, metadata: &[u8]) -> ZingResult<()> {
        let cf = self.db.cf_handle(METADATA_CF)
            .ok_or_else(|| ZingError::Cache("metadata CF not found".into()))?;
        self.db.put_cf(&cf, blob_id.as_bytes(), metadata)
            .map_err(|e| ZingError::Cache(e.to_string()))?;
        Ok(())
    }

    pub fn get_metadata(&self, blob_id: &str) -> ZingResult<Option<Vec<u8>>> {
        let cf = self.db.cf_handle(METADATA_CF)
            .ok_or_else(|| ZingError::Cache("metadata CF not found".into()))?;
        let result = self.db.get_cf(&cf, blob_id.as_bytes())
            .map_err(|e| ZingError::Cache(e.to_string()))?;
        Ok(result)
    }

    pub fn list_blob_ids(&self) -> ZingResult<Vec<String>> {
        let cf = self.db.cf_handle(BLOBS_CF)
            .ok_or_else(|| ZingError::Cache("blobs CF not found".into()))?;
        let iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
        let mut ids = Vec::new();
        for item in iter {
            let (key, _) = item.map_err(|e| ZingError::Cache(e.to_string()))?;
            let id = String::from_utf8(key.to_vec())
                .map_err(|e| ZingError::Cache(e.to_string()))?;
            ids.push(id);
        }
        Ok(ids)
    }

    pub fn blob_size(&self, blob_id: &str) -> ZingResult<Option<u64>> {
        let data = self.get(blob_id)?;
        Ok(data.map(|d| d.len() as u64))
    }

    pub fn total_size(&self) -> ZingResult<u64> {
        let ids = self.list_blob_ids()?;
        let mut total: u64 = 0;
        for id in ids {
            if let Some(size) = self.blob_size(&id)? {
                total += size;
            }
        }
        Ok(total)
    }

    pub fn db(&self) -> &DB {
        &self.db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (BlobStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = BlobStore::open(dir.path()).expect("open store");
        (store, dir)
    }

    #[test]
    fn test_store_and_retrieve_blob() {
        let (store, _dir) = temp_store();
        let data = b"hello world".to_vec();
        store.put("test_blob_123", &data).expect("put blob");
        let retrieved = store.get("test_blob_123").expect("get blob").expect("blob should exist");
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_store_returns_none_for_missing_blob() {
        let (store, _dir) = temp_store();
        let result = store.get("nonexistent").expect("get should not error");
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_blob() {
        let (store, _dir) = temp_store();
        let data = b"hello world".to_vec();
        store.put("test_blob_123", &data).expect("put blob");
        store.delete("test_blob_123").expect("delete blob");
        let result = store.get("test_blob_123").expect("get should not error");
        assert!(result.is_none());
    }

    #[test]
    fn test_blob_size() {
        let (store, _dir) = temp_store();
        let data = b"hello world".to_vec();
        store.put("test_blob_123", &data).expect("put blob");
        let size = store.blob_size("test_blob_123").expect("size").expect("should have size");
        assert_eq!(size, 11);
    }

    #[test]
    fn test_total_size() {
        let (store, _dir) = temp_store();
        store.put("blob_a", b"12345").expect("put");
        store.put("blob_b", b"67890").expect("put");
        let total = store.total_size().expect("total");
        assert_eq!(total, 10);
    }

    #[test]
    fn test_metadata() {
        let (store, _dir) = temp_store();
        let meta = b"metadata_bytes".to_vec();
        store.put_metadata("test_blob", &meta).expect("put metadata");
        let retrieved = store.get_metadata("test_blob").expect("get metadata").expect("should exist");
        assert_eq!(retrieved, meta);
    }
}