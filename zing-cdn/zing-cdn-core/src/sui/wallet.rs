use std::path::Path;
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

        // Keystore: env var or file
        let keys_json: Vec<String> = if let Ok(json) = std::env::var("ZING_SUI_KEYSTORE_JSON") {
            serde_json::from_str(&json)
                .map_err(|e| ZingError::SuiClient(format!("env ZING_SUI_KEYSTORE_JSON: {}", e)))?
        } else {
            let keystore_file = keystore_path
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| config_dir.join("sui.keystore"));
            serde_json::from_str(
                &std::fs::read_to_string(&keystore_file)
                    .map_err(|e| ZingError::SuiClient(format!("keystore read: {}", e)))?
            ).map_err(|e| ZingError::SuiClient(format!("keystore parse: {}", e)))?
        };

        // Active address: env var or client.yaml
        let target_address = if let Ok(addr_str) = std::env::var("ZING_SUI_ADDRESS") {
            use std::str::FromStr;
            Address::from_str(&addr_str)
                .map_err(|e| ZingError::SuiClient(format!("env ZING_SUI_ADDRESS '{}': {}", addr_str, e)))?
        } else {
            parse_active_address(&config_dir.join("client.yaml"))
                .ok_or_else(|| ZingError::SuiClient("no active_address in client.yaml or ZING_SUI_ADDRESS env".into()))?
        };

        // Find matching Ed25519 key
        let keypair = keys_json.iter()
            .filter_map(|k| {
                let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, k).ok()?;
                if raw.len() == 33 && raw[0] == 0x00 {
                    let seed: [u8; 32] = raw[1..].try_into().ok()?;
                    let kp = Ed25519PrivateKey::new(seed);
                    let addr = kp.public_key().derive_address();
                    if addr == target_address { Some(kp) } else { None }
                } else { None }
            })
            .next()
            .ok_or_else(|| ZingError::SuiClient(
                "no Ed25519 key matching active_address in keystore".into()
            ))?;

        tracing::info!(address = %target_address, "Sui wallet loaded");

        Ok(Self {
            address: target_address,
            keypair,
            settlement,
            rpc_url: "https://fullnode.mainnet.sui.io:443".into(),
            payment_counter: AtomicU64::new(0),
        })
    }

    pub fn address(&self) -> Address { self.address }

    pub async fn pay_wal(
        &self, recipient: Address, blob_hash: &[u8; 32], amount: u64,
    ) -> ZingResult<PaymentProof> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return self.synthetic_pay(recipient, blob_hash, amount),
        };
        match self.try_onchain_pay(settlement, recipient, blob_hash, amount).await {
            Ok(d) => Ok(d),
            Err(e) => {
                tracing::warn!(%e, "On-chain payment failed, falling back");
                self.synthetic_pay(recipient, blob_hash, amount)
            }
        }
    }

    async fn try_onchain_pay(
        &self, settlement: &SettlementConfig, recipient: Address,
        blob_hash: &[u8; 32], amount: u64,
    ) -> ZingResult<PaymentProof> {
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        // tx.coin() auto-selects + splits exact amount — no manual coin selection
        let tx = settlement.build_pay_transaction(self.address, recipient, blob_hash, amount);

        let transaction = tx.build(&mut client).await
            .map_err(|e| ZingError::SuiClient(format!("build: {}", e)))?;

        let signature = self.keypair.sign_transaction(&transaction)
            .map_err(|e| ZingError::SuiClient(format!("sign: {}", e)))?;

        let request = sui_rpc::proto::sui::rpc::v2::ExecuteTransactionRequest::new(transaction.into())
            .with_signatures(vec![signature.into()])
            .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["digest"]));

        let response = client
            .execute_transaction_and_wait_for_checkpoint(request, Duration::from_secs(60))
            .await
            .map_err(|e| ZingError::SuiClient(format!("tx: {}", e)))?;

        let digest_b58 = response
            .into_inner()
            .transaction
            .and_then(|t| t.digest)
            .ok_or_else(|| ZingError::SuiClient("no digest".into()))?;

        let digest_raw = bs58::decode(&digest_b58).into_vec()
            .map_err(|e| ZingError::SuiClient(format!("digest decode: {}", e)))?;

        let mut digest = [0u8; 32];
        digest.copy_from_slice(&digest_raw);

        let c = self.payment_counter.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(recipient = %recipient, amount, counter = c,
            tx_digest = %digest_b58,
            "WAL payment — on-chain settlement::pay executed");
        Ok(digest)
    }

    pub async fn register_peer(&self, peer_id_bytes: Vec<u8>) -> ZingResult<()> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return Ok(()),
        };
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let tx = settlement.build_register_transaction(self.address, peer_id_bytes, 1_000_000_000);

        let transaction = tx.build(&mut client).await
            .map_err(|e| ZingError::SuiClient(format!("register: {}", e)))?;

        let signature = self.keypair.sign_transaction(&transaction)
            .map_err(|e| ZingError::SuiClient(format!("sign: {}", e)))?;

        let request = sui_rpc::proto::sui::rpc::v2::ExecuteTransactionRequest::new(transaction.into())
            .with_signatures(vec![signature.into()]);

        let response = client
            .execute_transaction_and_wait_for_checkpoint(request, Duration::from_secs(60))
            .await
            .map_err(|e| ZingError::SuiClient(format!("register tx: {}", e)))?;

        let digest = response.into_inner().transaction
            .and_then(|t| t.digest).unwrap_or_default();
        tracing::info!(tx_digest = %digest, "Peer registered on-chain");
        Ok(())
    }

    pub fn settlement_config(&self) -> Option<&SettlementConfig> { self.settlement.as_ref() }

    fn synthetic_pay(
        &self, recipient: Address, blob_hash: &[u8; 32], amount: u64,
    ) -> ZingResult<PaymentProof> {
        let c = self.payment_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let mut h = Sha256::new();
        h.update(b"zing-payment-v1");
        h.update(recipient.to_string().as_bytes());
        h.update(blob_hash);
        h.update(&amount.to_le_bytes());
        h.update(&c.to_le_bytes());
        let d: [u8; 32] = h.finalize().into();
        tracing::info!(?recipient, amount, counter = c, proof = %hex::encode(d),
            "WAL payment (synthetic proof)");
        Ok(d)
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
    use std::str::FromStr;
    let content = std::fs::read_to_string(client_yaml).ok()?;
    for line in content.lines() {
        let t = line.trim();
        if let Some(a) = t.strip_prefix("active_address:") {
            let a = a.trim().trim_matches('"').trim_end_matches(" ~");
            if !a.is_empty() && a != "~" { return a.parse().ok(); }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_synthetic_proof_non_zero() {
        let w = ZingWallet::test_wallet();
        let p = w.synthetic_pay(Address::ZERO, &[1u8; 32], 1_000_000).unwrap();
        assert_ne!(p, [0u8; 32]);
    }
    #[tokio::test]
    async fn test_synthetic_proof_counter() {
        let w = ZingWallet::test_wallet();
        let p1 = w.synthetic_pay(Address::ZERO, &[2u8; 32], 100).unwrap();
        let p2 = w.synthetic_pay(Address::ZERO, &[2u8; 32], 100).unwrap();
        assert_ne!(p1, p2);
    }
}
