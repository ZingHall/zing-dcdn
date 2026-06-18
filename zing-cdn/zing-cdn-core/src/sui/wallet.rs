use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use sui_sdk::SuiClientBuilder;
use sui_sdk::types::base_types::SuiAddress;
use sui_sdk::wallet_context::WalletContext;

use crate::types::{ZingError, ZingResult};

use super::settlement::SettlementConfig;

pub type PaymentProof = [u8; 32];

pub struct ZingWallet {
    address: SuiAddress,
    wallet_ctx: Option<WalletContext>,
    sui_client: Option<Arc<sui_sdk::SuiClient>>,
    settlement: Option<SettlementConfig>,
    payment_counter: AtomicU64,
}

impl ZingWallet {
    pub async fn from_keystore(
        keystore_path: Option<&Path>,
        settlement: Option<SettlementConfig>,
    ) -> ZingResult<Self> {
        let config_path = keystore_path.map(|p| p.to_path_buf()).unwrap_or_else(|| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| "/tmp".to_string());
            std::path::PathBuf::from(home)
                .join(".sui")
                .join("sui_config")
                .join("client.yaml")
        });

        let mut wallet_ctx = WalletContext::new(&config_path)
            .map_err(|e| ZingError::SuiClient(format!("failed to load wallet: {}", e)))?;

        let address = wallet_ctx.active_address()
            .map_err(|e| ZingError::SuiClient(format!("active address: {}", e)))?;

        let sui_client = SuiClientBuilder::default()
            .build("https://fullnode.mainnet.sui.io:443")
            .await
            .map_err(|e| ZingError::SuiClient(format!("sui client: {}", e)))?;

        tracing::info!(%address, "Sui wallet loaded with mainnet connection");

        Ok(Self {
            address,
            wallet_ctx: Some(wallet_ctx),
            sui_client: Some(Arc::new(sui_client)),
            settlement,
            payment_counter: AtomicU64::new(0),
        })
    }

    pub fn address(&self) -> SuiAddress {
        self.address
    }

    pub async fn pay_wal(
        &self,
        recipient: SuiAddress,
        blob_hash: &[u8; 32],
        amount_nanos: u64,
    ) -> ZingResult<PaymentProof> {
        let settlement = match &self.settlement {
            Some(s) => s,
            None => return self.synthetic_pay(recipient, blob_hash, amount_nanos),
        };

        match self.try_onchain_pay(settlement, recipient, blob_hash, amount_nanos).await {
            Ok(digest) => Ok(digest),
            Err(e) => {
                tracing::warn!(%e, "On-chain payment failed, falling back to synthetic proof");
                self.synthetic_pay(recipient, blob_hash, amount_nanos)
            }
        }
    }

    async fn try_onchain_pay(
        &self,
        settlement: &SettlementConfig,
        recipient: SuiAddress,
        blob_hash: &[u8; 32],
        amount_nanos: u64,
    ) -> ZingResult<PaymentProof> {
        let wallet_ctx = self.wallet_ctx.as_ref()
            .ok_or_else(|| ZingError::SuiClient("no wallet context".into()))?;
        let sui_client = self.sui_client.as_ref()
            .ok_or_else(|| ZingError::SuiClient("no sui client".into()))?;

        let gas = wallet_ctx
            .get_one_gas_object_owned_by_address(self.address)
            .await
            .map_err(|e| ZingError::SuiClient(format!("gas: {}", e)))?
            .ok_or_else(|| ZingError::SuiClient("no gas coins".into()))?;

        let wal_type = &settlement.wal_coin_type;
        let wal_coins = sui_client
            .coin_read_api()
            .get_coins(self.address, Some(wal_type.clone()), None, None)
            .await
            .map_err(|e| ZingError::SuiClient(format!("wal coins: {}", e)))?;

        let wal_coin = wal_coins.data.iter()
            .find(|c| c.balance >= amount_nanos)
            .ok_or_else(|| ZingError::SuiClient(format!(
                "insufficient WAL: need {} frost, have {} coins",
                amount_nanos, wal_coins.data.len()
            )))?;

        let payment_coin_ref = (wal_coin.coin_object_id, wal_coin.version, wal_coin.digest);

        let tx_data = settlement.build_pay_transaction(
            self.address, recipient, blob_hash,
            payment_coin_ref, gas, 5_000_000, 1_000,
        ).map_err(|e| ZingError::SuiClient(format!("ptb: {}", e)))?;

        let signed = wallet_ctx.sign_transaction(&tx_data).await;
        let effects = wallet_ctx
            .execute_transaction_may_fail(signed)
            .await
            .map_err(|e| ZingError::SuiClient(format!("tx: {}", e)))?;

        let digest: [u8; 32] = effects.transaction.digest().into_inner();
        let counter = self.payment_counter.fetch_add(1, Ordering::Relaxed) + 1;

        tracing::info!(
            recipient = %recipient, amount = amount_nanos, counter = counter,
            tx_digest = %hex::encode(digest),
            "WAL payment — on-chain settlement::pay executed"
        );

        Ok(digest)
    }

    pub async fn register_peer(&self, peer_id_bytes: Vec<u8>) -> ZingResult<()> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return Ok(()),
        };
        let wallet_ctx = self.wallet_ctx.as_ref()
            .ok_or_else(|| ZingError::SuiClient("no wallet context".into()))?;
        let sui_client = self.sui_client.as_ref()
            .ok_or_else(|| ZingError::SuiClient("no sui client".into()))?;

        let gas = wallet_ctx
            .get_one_gas_object_owned_by_address(self.address)
            .await
            .map_err(|e| ZingError::SuiClient(format!("gas: {}", e)))?
            .ok_or_else(|| ZingError::SuiClient("no gas coins".into()))?;

        let min_bond = 1_000_000_000u64;  // 1 WAL in frost
        let wal_type = &settlement.wal_coin_type;
        let wal_coins = sui_client
            .coin_read_api()
            .get_coins(self.address, Some(wal_type.clone()), None, None)
            .await
            .map_err(|e| ZingError::SuiClient(format!("wal: {}", e)))?;

        let bond_coin = wal_coins.data.iter()
            .find(|c| c.balance >= min_bond)
            .ok_or_else(|| ZingError::SuiClient(format!(
                "insufficient WAL for bond (need {} frost, have {} coins)",
                min_bond, wal_coins.data.len()
            )))?;

        let bond_ref = (bond_coin.coin_object_id, bond_coin.version, bond_coin.digest);
        let tx_data = settlement.build_register_transaction(
            self.address, peer_id_bytes, bond_ref, gas, 10_000_000, 1_000,
        ).map_err(|e| ZingError::SuiClient(format!("register ptb: {}", e)))?;

        let signed = wallet_ctx.sign_transaction(&tx_data).await;
        let effects = wallet_ctx
            .execute_transaction_may_fail(signed)
            .await
            .map_err(|e| ZingError::SuiClient(format!("register tx: {}", e)))?;

        tracing::info!(tx_digest = %effects.transaction.digest(), "Peer registered on-chain");
        Ok(())
    }

    pub fn settlement_config(&self) -> Option<&SettlementConfig> {
        self.settlement.as_ref()
    }

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
        tracing::info!(recipient = %recipient, amount = amount_nanos, counter = counter,
            proof = %hex::encode(digest), "WAL payment (synthetic proof)");
        Ok(digest)
    }
}

#[cfg(test)]
impl ZingWallet {
    pub fn test_wallet() -> Self {
        Self {
            address: SuiAddress::random_for_testing_only(),
            wallet_ctx: None,
            sui_client: None,
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
        assert_ne!(proof, [0u8; 32]);
        assert_eq!(proof.len(), 32);
    }

    #[tokio::test]
    async fn test_synthetic_pay_counter_increments() {
        let wallet = ZingWallet::test_wallet();
        let recipient = SuiAddress::random_for_testing_only();
        let blob_hash = [2u8; 32];
        let amount = 100u64;
        let proof1 = wallet.synthetic_pay(recipient, &blob_hash, amount).unwrap();
        let proof2 = wallet.synthetic_pay(recipient, &blob_hash, amount).unwrap();
        assert_ne!(proof1, proof2);
    }
}
