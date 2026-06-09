use std::sync::Arc;
use std::time::Instant;

use walrus_core::encoding::EncodingConfig;
use walrus_core::encoding::EncodingFactory;
use walrus_core::BlobId;
use walrus_core::EncodingType;
use walrus_core::metadata::{BlobMetadataApi, VerifiedBlobMetadataWithId};

use crate::types::{ZingError, ZingResult};

pub struct BlobVerifier {
    encoding_config: Arc<EncodingConfig>,
}

impl BlobVerifier {
    pub fn new(encoding_config: Arc<EncodingConfig>) -> Self {
        Self { encoding_config }
    }

    pub fn verify_blob_by_id(
        &self,
        expected_blob_id: &BlobId,
        blob_data: &[u8],
    ) -> ZingResult<()> {
        let t0 = Instant::now();
        let factory = self.encoding_config.get_for_type(EncodingType::RS2);
        eprintln!("L1: verify — get_for_type in {:?}", t0.elapsed());

        let t_compute = Instant::now();
        let computed_blob_id = factory
            .compute_blob_id(blob_data)
            .map_err(|e| ZingError::WalrusClient(format!("compute_blob_id failed: {e}")))?;
        eprintln!("L1: verify — compute_blob_id ({} bytes) in {:?}", blob_data.len(), t_compute.elapsed());

        if &computed_blob_id != expected_blob_id {
            return Err(ZingError::VerificationFailed {
                computed: computed_blob_id.to_string(),
                expected: expected_blob_id.to_string(),
            });
        }

        eprintln!("L1: verify — total {:?}", t0.elapsed());
        Ok(())
    }

    pub fn verify_blob_against_metadata(
        &self,
        metadata: &VerifiedBlobMetadataWithId,
        blob_data: &[u8],
    ) -> ZingResult<()> {
        if !self.quick_size_check(metadata, blob_data) {
            return Err(ZingError::VerificationFailed {
                computed: format!("size={}", blob_data.len()),
                expected: format!("size={}", metadata.metadata().unencoded_length()),
            });
        }

        let expected_blob_id = metadata.blob_id();
        let encoding_type = metadata.metadata().encoding_type();
        let factory = self.encoding_config.get_for_type(encoding_type);

        let computed_blob_id = factory
            .compute_blob_id(blob_data)
            .map_err(|e| ZingError::WalrusClient(format!("compute_blob_id failed: {e}")))?;

        if &computed_blob_id != expected_blob_id {
            return Err(ZingError::VerificationFailed {
                computed: computed_blob_id.to_string(),
                expected: expected_blob_id.to_string(),
            });
        }

        Ok(())
    }

    pub fn quick_size_check(&self, metadata: &VerifiedBlobMetadataWithId, blob_data: &[u8]) -> bool {
        blob_data.len() == metadata.metadata().unencoded_length() as usize
    }
}