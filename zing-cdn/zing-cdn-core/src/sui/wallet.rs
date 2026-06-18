use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use sui_sdk::types::base_types::SuiAddress;
use walrus_sui::config::load_wallet_context_from_path;

use crate::types::{ZingError, ZingResult};

/// A unique payment reference (simulated tx digest for MVP).
pub type PaymentProof = [u8; 32];

/// Wraps a Sui wallet for WAL token payments between peers.
///
/// MVP: The wallet is loaded and Sui address is available, but actual
/// on-chain WAL transfers are stubbed (logged as payment intent).
/// The full Sui transaction execution will be added in a follow-up
/// using sui-sdk's SuiClient for coin transfers.
pub struct ZingWallet {
    address: SuiAddress,
    payment_counter: AtomicU64,
}

impl ZingWallet {
    /// Load a wallet from a Sui CLI keystore directory (typically ~/.sui/sui_config/).
    pub async fn from_keystore(
        keystore_path: &Path,
    ) -> ZingResult<Self> {
        let wallet = load_wallet_context_from_path(Some(keystore_path), None)
            .map_err(|e| ZingError::SuiClient(format!("failed to load wallet: {}", e)))?;

        let address = wallet.active_address();

        tracing::info!(%address, "Sui wallet loaded successfully");

        Ok(Self { address, payment_counter: AtomicU64::new(0) })
    }

    /// Returns this wallet's Sui address (32 bytes).
    pub fn address(&self) -> SuiAddress {
        self.address
    }

    /// Generates a payment proof for a WAL transfer.
    ///
    /// MVP: Does not execute an on-chain transaction. Instead, generates
    /// a synthetic payment proof (a hash of the payment details) that
    /// the serving peer can log/verify. On-chain execution will be
    /// added in a follow-up.
    pub async fn pay_wal(&self, recipient: SuiAddress, amount_nanos: u64) -> ZingResult<PaymentProof> {
        let counter = self.payment_counter.fetch_add(1, Ordering::Relaxed) + 1;

        // Generate a synthetic proof: SHA256(recipient || amount || counter)
        let mut hasher = Sha256::new();
        hasher.update(b"zing-payment-v1");
        hasher.update(&recipient.to_vec());
        hasher.update(&amount_nanos.to_le_bytes());
        hasher.update(&counter.to_le_bytes());
        let digest: [u8; 32] = hasher.finalize().into();

        tracing::info!(
            recipient = %recipient,
            amount = amount_nanos,
            counter = counter,
            proof = %hex::encode(digest),
            "WAL payment (MVP: synthetic proof — on-chain tx pending)"
        );

        Ok(digest)
    }
}

#[cfg(test)]
impl ZingWallet {
    pub fn test_wallet() -> Self {
        Self {
            address: SuiAddress::random_for_testing_only(),
            payment_counter: AtomicU64::new(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pay_wal_returns_non_zero_proof() {
        let wallet = ZingWallet::test_wallet();
        let recipient = SuiAddress::random_for_testing_only();
        let amount = 1_000_000u64;

        let proof = wallet.pay_wal(recipient, amount).await.unwrap();

        assert_ne!(proof, [0u8; 32], "payment proof must not be all zeros");
        assert_eq!(proof.len(), 32, "payment proof must be 32 bytes");
    }

    #[tokio::test]
    async fn test_pay_wal_counter_increments() {
        let wallet = ZingWallet::test_wallet();
        let recipient = SuiAddress::random_for_testing_only();
        let amount = 100u64;

        let proof1 = wallet.pay_wal(recipient, amount).await.unwrap();
        let proof2 = wallet.pay_wal(recipient, amount).await.unwrap();

        assert_ne!(proof1, proof2, "payment proofs for different counters must differ");
    }
}
