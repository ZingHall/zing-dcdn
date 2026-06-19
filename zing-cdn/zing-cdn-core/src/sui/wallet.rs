use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use sui_crypto::ed25519::Ed25519PrivateKey;
use sui_crypto::SuiSigner;
use sui_rpc::field::FieldMaskUtil;
use sui_sdk_types::Address;

use crate::types::{ZingError, ZingResult};

use super::settlement::SettlementConfig;

pub type PaymentProof = [u8; 32];

pub struct ZingWallet {
    address: Address,
    keypair: Ed25519PrivateKey,
    settlement: Option<SettlementConfig>,
    rpc_url: String,
    payment_counter: AtomicU64,
}

impl ZingWallet {
    pub async fn from_keystore(
        keystore_path: Option<&Path>,
        settlement: Option<SettlementConfig>,
    ) -> ZingResult<Self> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let config_dir = std::path::PathBuf::from(&home).join(".sui").join("sui_config");

        // Load keystore
        let keystore_file = keystore_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| config_dir.join("sui.keystore"));

        let keys_json: Vec<String> = serde_json::from_str(
            &std::fs::read_to_string(&keystore_file)
                .map_err(|e| ZingError::SuiClient(format!("keystore read: {}", e)))?
        ).map_err(|e| ZingError::SuiClient(format!("keystore parse: {}", e)))?;

        // Get target address from client.yaml
        let config_yaml = config_dir.join("client.yaml");
        let target_address = parse_active_address(&config_yaml)
            .ok_or_else(|| ZingError::SuiClient("no active_address in client.yaml".into()))?;

        // Find Ed25519 key whose public key derives to the active address
        let keypair = keys_json.iter()
            .filter_map(|k| {
                let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, k).ok()?;
                if raw.len() == 33 && raw[0] == 0x00 {
                    let seed: [u8; 32] = raw[1..].try_into().ok()?;
                    let kp = Ed25519PrivateKey::new(seed);
                    let addr = kp.public_key().derive_address();
                    if addr == target_address { Some(kp) } else { None }
                } else {
                    None
                }
            })
            .next()
            .ok_or_else(|| ZingError::SuiClient(
                "no Ed25519 key matching active_address in keystore".into()
            ))?;

        let address = target_address;

        tracing::info!(%address, "Sui wallet loaded (new SDK)");

        Ok(Self {
            address,
            keypair,
            settlement,
            rpc_url: "https://fullnode.mainnet.sui.io:443".into(),
            payment_counter: AtomicU64::new(0),
        })
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub async fn pay_wal(
        &self,
        recipient: Address,
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
        recipient: Address,
        blob_hash: &[u8; 32],
        amount_nanos: u64,
    ) -> ZingResult<PaymentProof> {
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc client: {}", e)))?;

        // Select WAL payment coin
        let wal_type = sui_sdk_types::TypeTag::from_str(&settlement.wal_coin_type)
            .map_err(|e| ZingError::SuiClient(format!("wal type: {}", e)))?;
        let wal_coins = client
            .select_coins(&self.address, &wal_type, amount_nanos, &[])
            .await
            .map_err(|e| ZingError::SuiClient(format!("wal coins: {}", e)))?;

        let wal_coin = wal_coins.first()
            .ok_or_else(|| ZingError::SuiClient("no WAL coins found".into()))?;

        let coin_id = Address::from_str(wal_coin.object_id())
            .map_err(|e| ZingError::SuiClient(format!("coin id: {}", e)))?;
        let coin_digest = sui_sdk_types::Digest::from_str(wal_coin.digest())
            .map_err(|e| ZingError::SuiClient(format!("coin digest: {}", e)))?;

        // Build PTB
        let tx = settlement.build_pay_transaction(
            self.address,
            recipient,
            blob_hash,
            (coin_id, wal_coin.version(), coin_digest),
            5_000_000,
        );

        // Build (auto-selects gas)
        let transaction = tx
            .build(&mut client)
            .await
            .map_err(|e| ZingError::SuiClient(format!("tx build: {}", e)))?;

        // Sign
        let signature = self.keypair
            .sign_transaction(&transaction)
            .map_err(|e| ZingError::SuiClient(format!("sign: {}", e)))?;

        // Submit
        let request = sui_rpc::proto::sui::rpc::v2::ExecuteTransactionRequest::new(transaction.into())
            .with_signatures(vec![signature.into()])
            .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["digest"]));

        let response = client
            .execute_transaction_and_wait_for_checkpoint(request, Duration::from_secs(60))
            .await
            .map_err(|e| ZingError::SuiClient(format!("tx submit: {}", e)))?
            .into_inner();

        let digest_str = response
            .transaction
            .as_ref()
            .and_then(|t| t.digest.clone())
            .ok_or_else(|| ZingError::SuiClient("no digest in response".into()))?;

        let mut digest = [0u8; 32];
        digest.copy_from_slice(&hex::decode(&digest_str)
            .map_err(|e| ZingError::SuiClient(format!("digest decode: {}", e)))?);

        let counter = self.payment_counter.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(
            recipient = %recipient, amount = amount_nanos, counter = counter,
            tx_digest = %digest_str,
            "WAL payment — on-chain settlement::pay executed"
        );

        Ok(digest)
    }

    pub async fn register_peer(&self, peer_id_bytes: Vec<u8>) -> ZingResult<()> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return Ok(()),
        };

        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let min_bond = 1_000_000_000u64;
        let wal_type = sui_sdk_types::TypeTag::from_str(&settlement.wal_coin_type)
            .map_err(|e| ZingError::SuiClient(format!("wal type: {}", e)))?;
        let wal_coins = client
            .select_coins(&self.address, &wal_type, min_bond, &[])
            .await
            .map_err(|e| ZingError::SuiClient(format!("wal coins: {}", e)))?;

        let bond_coin = wal_coins.first()
            .ok_or_else(|| ZingError::SuiClient("no WAL coins for bond".into()))?;

        let coin_id = Address::from_str(bond_coin.object_id())
            .map_err(|e| ZingError::SuiClient(format!("coin id: {}", e)))?;
        let coin_digest = sui_sdk_types::Digest::from_str(bond_coin.digest())
            .map_err(|e| ZingError::SuiClient(format!("coin digest: {}", e)))?;

        let tx = settlement.build_register_transaction(
            self.address,
            peer_id_bytes,
            (coin_id, bond_coin.version(), coin_digest),
            10_000_000,
        );

        let transaction = tx.build(&mut client).await
            .map_err(|e| ZingError::SuiClient(format!("register build: {}", e)))?;

        let signature = self.keypair.sign_transaction(&transaction)
            .map_err(|e| ZingError::SuiClient(format!("sign: {}", e)))?;

        let request = sui_rpc::proto::sui::rpc::v2::ExecuteTransactionRequest::new(transaction.into())
            .with_signatures(vec![signature.into()]);

        let response = client
            .execute_transaction_and_wait_for_checkpoint(request, Duration::from_secs(60))
            .await
            .map_err(|e| ZingError::SuiClient(format!("register tx: {}", e)))?
            .into_inner();

        let digest_str = response.transaction.as_ref()
            .and_then(|t| t.digest.clone())
            .unwrap_or_default();

        tracing::info!(tx_digest = %digest_str, "Peer registered on-chain");
        Ok(())
    }

    pub fn settlement_config(&self) -> Option<&SettlementConfig> {
        self.settlement.as_ref()
    }

    fn synthetic_pay(
        &self,
        recipient: Address,
        blob_hash: &[u8; 32],
        amount_nanos: u64,
    ) -> ZingResult<PaymentProof> {
        let counter = self.payment_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let mut hasher = Sha256::new();
        hasher.update(b"zing-payment-v1");
        hasher.update(recipient.to_string().as_bytes());
        hasher.update(blob_hash);
        hasher.update(&amount_nanos.to_le_bytes());
        hasher.update(&counter.to_le_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        tracing::info!(?recipient, amount = amount_nanos, counter = counter,
            proof = %hex::encode(digest), "WAL payment (synthetic proof)");
        Ok(digest)
    }
}

#[cfg(test)]
impl ZingWallet {
    pub fn test_wallet() -> Self {
        Self {
            address: Address::ZERO,
            keypair: Ed25519PrivateKey::new([0u8; 32]),
            settlement: None,
            rpc_url: String::new(),
            payment_counter: AtomicU64::new(0),
        }
    }
}

fn parse_active_address(client_yaml: &Path) -> Option<Address> {
    let content = std::fs::read_to_string(client_yaml).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(addr_str) = trimmed.strip_prefix("active_address:") {
            let addr_str = addr_str.trim().trim_matches('"').trim_end_matches(" ~");
            if !addr_str.is_empty() && addr_str != "~" {
                return addr_str.parse().ok();
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_synthetic_pay_returns_non_zero_proof() {
        let wallet = ZingWallet::test_wallet();
        let recipient = Address::ZERO;
        let blob_hash = [1u8; 32];
        let amount = 1_000_000u64;
        let proof = wallet.synthetic_pay(recipient, &blob_hash, amount).unwrap();
        assert_ne!(proof, [0u8; 32]);
        assert_eq!(proof.len(), 32);
    }

    #[tokio::test]
    async fn test_synthetic_pay_counter_increments() {
        let wallet = ZingWallet::test_wallet();
        let recipient = Address::ZERO;
        let blob_hash = [2u8; 32];
        let amount = 100u64;
        let proof1 = wallet.synthetic_pay(recipient, &blob_hash, amount).unwrap();
        let proof2 = wallet.synthetic_pay(recipient, &blob_hash, amount).unwrap();
        assert_ne!(proof1, proof2);
    }
}
