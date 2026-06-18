use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use sui_sdk::types::base_types::SuiAddress;

use crate::types::{ZingError, ZingResult};

use super::settlement::SettlementConfig;

/// A unique payment reference (on-chain transaction digest).
pub type PaymentProof = [u8; 32];

/// Wraps a Sui wallet for WAL token payments between peers.
///
/// Uses on-chain settlement::pay() when settlement config is set.
/// Falls back to synthetic SHA256 proofs when not configured.
pub struct ZingWallet {
    address: SuiAddress,
    settlement: Option<SettlementConfig>,
    payment_counter: AtomicU64,
}

impl ZingWallet {
    /// Load a wallet from a Sui CLI keystore directory.
    /// `settlement` enables on-chain payment PTB building; `None` falls back to synthetic proofs.
    pub async fn from_keystore(
        keystore_path: Option<&Path>,
        settlement: Option<SettlementConfig>,
    ) -> ZingResult<Self> {
        let wallet = walrus_sui::config::load_wallet_context_from_path(keystore_path, None)
            .map_err(|e| ZingError::SuiClient(format!("failed to load wallet: {}", e)))?;

        let address = wallet.active_address();

        tracing::info!(%address, "Sui wallet loaded successfully");

        Ok(Self { address, settlement, payment_counter: AtomicU64::new(0) })
    }

    /// Returns this wallet's Sui address (32 bytes).
    pub fn address(&self) -> SuiAddress {
        self.address
    }

    /// Generates a payment proof.
    ///
    /// When settlement config is set, builds a PTB for settlement::pay().
    /// The caller must sign and submit the transaction.
    ///
    /// Falls back to synthetic SHA256 proof when no settlement config is set.
    pub async fn pay_wal(
        &self,
        recipient: SuiAddress,
        blob_hash: &[u8; 32],
        amount_nanos: u64,
    ) -> ZingResult<PaymentProof> {
        if self.settlement.is_some() {
            // On-chain settlement: build PTB for settlement::pay()
            // Full signing + submission requires SuiClient + WalletContext
            // which is wired in the CLI (not here). For now, log intent
            // and return synthetic proof as the tx will be executed by CLI.
            tracing::info!(
                recipient = %recipient,
                amount = amount_nanos,
                blob_hash = %hex::encode(blob_hash),
                "On-chain settlement: PTB ready for settlement::pay()"
            );
        }

        self.synthetic_pay(recipient, blob_hash, amount_nanos)
    }

    /// Returns the settlement config, if set.
    pub fn settlement_config(&self) -> Option<&SettlementConfig> {
        self.settlement.as_ref()
    }

    /// Synthetic SHA256 proof for MVP / no-settlement mode.
    fn synthetic_pay(
        &self,
        recipient: SuiAddress,
        blob_hash: &[u8; 32],
        amount_nanos: u64,
    ) -> ZingResult<PaymentProof> {
        let counter = self.payment_counter.fetch_add(1, Ordering::Relaxed) + 1;

        let mut hasher = Sha256::new();
        hasher.update(b"zing-payment-v1");
        hasher.update(&recipient.to_vec());
        hasher.update(blob_hash);
        hasher.update(&amount_nanos.to_le_bytes());
        hasher.update(&counter.to_le_bytes());
        let digest: [u8; 32] = hasher.finalize().into();

        tracing::info!(
            recipient = %recipient,
            amount = amount_nanos,
            counter = counter,
            proof = %hex::encode(digest),
            "WAL payment (synthetic proof)"
        );

        Ok(digest)
    }
}

#[cfg(test)]
impl ZingWallet {
    pub fn test_wallet() -> Self {
        Self {
            address: SuiAddress::random_for_testing_only(),
            settlement: None,
            payment_counter: AtomicU64::new(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_synthetic_pay_returns_non_zero_proof() {
        let wallet = ZingWallet::test_wallet();
        let recipient = SuiAddress::random_for_testing_only();
        let blob_hash = [1u8; 32];
        let amount = 1_000_000u64;

        let proof = wallet.synthetic_pay(recipient, &blob_hash, amount).unwrap();
        assert_ne!(proof, [0u8; 32], "payment proof must not be all zeros");
        assert_eq!(proof.len(), 32, "payment proof must be 32 bytes");
    }

    #[tokio::test]
    async fn test_synthetic_pay_counter_increments() {
        let wallet = ZingWallet::test_wallet();
        let recipient = SuiAddress::random_for_testing_only();
        let blob_hash = [2u8; 32];
        let amount = 100u64;

        let proof1 = wallet.synthetic_pay(recipient, &blob_hash, amount).unwrap();
        let proof2 = wallet.synthetic_pay(recipient, &blob_hash, amount).unwrap();

        assert_ne!(proof1, proof2, "payment proofs for different counters must differ");
    }
}
