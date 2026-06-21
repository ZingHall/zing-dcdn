use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use libp2p::PeerId;
use sha2::{Digest, Sha256};
use sui_crypto::ed25519::Ed25519PrivateKey;
use sui_crypto::SuiSigner;
use sui_rpc::field::FieldMaskUtil;
use sui_rpc::field::FieldMask;
use sui_rpc::proto::sui::rpc::v2::{BatchGetObjectsRequest, GetObjectRequest, ListDynamicFieldsRequest, ListOwnedObjectsRequest};
use sui_sdk_types::Address;

use crate::types::{ZingError, ZingResult};

use super::settlement::SettlementConfig;

pub type PaymentProof = [u8; 32];

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub sui_address: String,
    pub peer_id_b58: String,
    pub bond: u64,
    pub is_active: bool,
    pub peer_object_id: String,
    pub vault: Option<PeerVaultInfo>,
}

#[derive(Debug, Clone)]
pub struct PeerVaultInfo {
    pub reserves: u64,
    pub total_shares: u64,
    pub commission_bps: u64,
    pub peer_earnings: u64,
    pub vault_object_id: String,
}

#[derive(Debug, Clone)]
pub struct ShareCertificateInfo {
    pub cert_object_id: String,
    pub cert_version: u64,
    pub vault_id: String,
    pub vault_address: String,
    pub shares: u64,
    pub estimated_value: u64,
}

#[derive(Debug, Clone)]
pub struct RegisteredPeerInfo {
    pub peer_id_bytes: Vec<u8>,
    pub peer_object_id: String,
    pub initial_shared_version: u64,
}

pub struct ZingWallet {
    address: Address,
    keypair: Ed25519PrivateKey,
    seed: [u8; 32],
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

        // Load keystore and derive address.
        // Priority: ZING_CDN_SUI_PRIVATE_KEY (single key), ZING_CDN_SUI_KEYSTORE_JSON (array), file.
        let (keypair, address, seed) = if let Ok(key) = std::env::var("ZING_CDN_SUI_PRIVATE_KEY") {
            decode_and_derive(vec![key])?
        } else if let Ok(json) = std::env::var("ZING_CDN_SUI_KEYSTORE_JSON") {
            let keys: Vec<String> = serde_json::from_str(&json)
                .map_err(|e| ZingError::SuiClient(format!("env ZING_CDN_SUI_KEYSTORE_JSON: {}", e)))?;
            decode_and_derive(keys)?
        } else {
            let keystore_file = keystore_path
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| config_dir.join("sui.keystore"));
            let keys: Vec<String> = serde_json::from_str(
                &std::fs::read_to_string(&keystore_file)
                    .map_err(|e| ZingError::SuiClient(format!("keystore read: {}", e)))?
            ).map_err(|e| ZingError::SuiClient(format!("keystore parse: {}", e)))?;

            let target = parse_active_address(&config_dir.join("client.yaml"))
                .ok_or_else(|| ZingError::SuiClient("no active_address in client.yaml".into()))?;

            decode_first_matching(keys, target)?
        };

        tracing::info!(address = %address, "Sui wallet loaded");

        Ok(Self {
            address,
            keypair,
            seed,
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

        // Look up the recipient's vault to route payment to the correct vault
        let vault_obj_id = self.get_vault_for_recipient(recipient).await?;

        // tx.coin() auto-selects + splits exact amount — no manual coin selection
        let tx = settlement.build_pay_transaction(self.address, recipient, blob_hash, amount, vault_obj_id);

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

    pub async fn is_peer_registered(&self) -> ZingResult<bool> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return Ok(false),
        };
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let peers_table_id = format!("0x{}", hex::encode(settlement.registry_peers_table_id));
        let address_bytes: [u8; 32] = self.address.into();

        let request = ListDynamicFieldsRequest::const_default()
            .with_parent(&peers_table_id)
            .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["name", "value"]));

        let stream = client.list_dynamic_fields(request);
        let mut stream = std::pin::pin!(stream);

        while let Some(result) = stream.next().await {
            match result {
                Ok(df) => {
                    if let Some(name_bcs) = &df.name {
                        if let Some(name_bytes) = &name_bcs.value {
                            if name_bytes.as_ref() == address_bytes {
                                let vault_id_hex = if let Some(child_id) = &df.child_id {
                                    child_id.clone()
                                } else if let Some(value_bcs) = &df.value {
                                    if let Some(value_bytes) = &value_bcs.value {
                                        format!("0x{}", hex::encode(value_bytes.as_ref()))
                                    } else {
                                        continue;
                                    }
                                } else {
                                    continue;
                                };
                                return Self::check_peer_active(&mut client, &vault_id_hex).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(%e, "Failed to list dynamic fields from peers table");
                }
            }
        }

        Ok(false)
    }

    async fn check_peer_active(
        client: &mut sui_rpc::Client, vault_id: &str,
    ) -> ZingResult<bool> {
        let response = client.ledger_client().get_object(
            GetObjectRequest::const_default()
                .with_object_id(vault_id)
                .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["json"])),
        ).await
        .map_err(|e| ZingError::SuiClient(format!("get vault: {}", e)))?;

        let object = response.into_inner().object
            .ok_or_else(|| ZingError::SuiClient("vault object not found".into()))?;

        if let Some(json) = object.json {
            let json_val: &prost_types::Value = &json;
            if let Some(prost_types::value::Kind::StructValue(s)) = json_val.kind.as_ref() {
                if let Some(is_active_val) = s.fields.get("is_active") {
                    if let Some(prost_types::value::Kind::BoolValue(b)) = is_active_val.kind.as_ref() {
                        return Ok(*b);
                    }
                }
            }
        }

        Ok(false)
    }

    pub async fn list_all_peers(&self) -> ZingResult<Vec<PeerInfo>> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return Ok(vec![]),
        };
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let peers_table_id = format!("0x{}", hex::encode(settlement.registry_peers_table_id));

        let request = ListDynamicFieldsRequest::const_default()
            .with_parent(&peers_table_id)
            .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["name", "value"]));

        let stream = client.list_dynamic_fields(request);
        let mut stream = std::pin::pin!(stream);

        let mut peer_object_ids: Vec<String> = Vec::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(df) => {
                    let object_id = if let Some(child_id) = &df.child_id {
                        child_id.clone()
                    } else if let Some(value_bcs) = &df.value {
                        if let Some(value_bytes) = &value_bcs.value {
                            format!("0x{}", hex::encode(value_bytes.as_ref()))
                        } else { continue; }
                    } else { continue; };

                    peer_object_ids.push(object_id);
                }
                Err(e) => {
                    tracing::warn!(%e, "Failed to list dynamic fields from peers table");
                }
            }
        }

        if peer_object_ids.is_empty() {
            return Ok(vec![]);
        }

        let requests: Vec<GetObjectRequest> = peer_object_ids
            .iter()
            .map(|id| GetObjectRequest::const_default().with_object_id(id))
            .collect();

        let response = client
            .ledger_client()
            .batch_get_objects(
                BatchGetObjectsRequest::const_default()
                    .with_requests(requests)
                    .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["json"])),
            )
            .await
            .map_err(|e| ZingError::SuiClient(format!("batch_get_objects: {}", e)))?;

        let results = response.into_inner().objects;

        let mut peers = Vec::new();
        for (obj_id, result) in peer_object_ids.into_iter().zip(results) {
            if let Some(obj) = result.result.and_then(|r| match r {
                sui_rpc::proto::sui::rpc::v2::get_object_result::Result::Object(o) => Some(o),
                _ => None,
            }) {
                if let Some(json) = &obj.json {
                    if let Some(info) = parse_peer_json(json, &obj_id) {
                        peers.push(info);
                    }
                }
            }
        }

        // Attach vault info to matching peers
        let vaults = self.list_peer_vaults().await.unwrap_or_default();
        for peer in &mut peers {
            let addr_normalized = peer.sui_address.trim_start_matches("0x").to_lowercase();
            peer.vault = vaults.iter().find_map(|(vault_addr, vault_info)| {
                let vault_addr_normalized = vault_addr.trim_start_matches("0x").to_lowercase();
                if vault_addr_normalized == addr_normalized {
                    Some(vault_info.clone())
                } else {
                    None
                }
            });
        }

        Ok(peers)
    }

    pub async fn get_wal_balance(&self) -> ZingResult<u64> {
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let wal_coin_type = "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL";
        let owner = format!("{:#}", self.address);

        let request = sui_rpc::proto::sui::rpc::v2::GetBalanceRequest::const_default()
            .with_owner(&owner)
            .with_coin_type(wal_coin_type);

        let response = client
            .state_client()
            .get_balance(request)
            .await
            .map_err(|e| ZingError::SuiClient(format!("get_balance: {}", e)))?;

        let balance = response.into_inner().balance
            .ok_or_else(|| ZingError::SuiClient("no balance in response".into()))?;

        Ok(balance.coin_balance.unwrap_or(0))
    }

    pub async fn list_peer_vaults(&self) -> ZingResult<HashMap<String, PeerVaultInfo>> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return Ok(HashMap::new()),
        };
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let vaults_table_id = format!("0x{}", hex::encode(settlement.peer_vaults_table_id));

        let request = ListDynamicFieldsRequest::const_default()
            .with_parent(&vaults_table_id)
            .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["name", "value"]));

        let stream = client.list_dynamic_fields(request);
        let mut stream = std::pin::pin!(stream);

        let mut addr_to_vault_id: Vec<(String, String)> = Vec::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(df) => {
                    let vault_id = if let Some(child_id) = &df.child_id {
                        child_id.clone()
                    } else if let Some(value_bcs) = &df.value {
                        if let Some(value_bytes) = &value_bcs.value {
                            format!("0x{}", hex::encode(value_bytes.as_ref()))
                        } else { continue; }
                    } else { continue; };

                    let addr = if let Some(name_bcs) = &df.name {
                        if let Some(name_bytes) = &name_bcs.value {
                            let bytes = name_bytes.as_ref();
                            if bytes.len() < 32 { continue; }
                            format!("0x{}", hex::encode(&bytes[..32]))
                        } else { continue; }
                    } else { continue; };

                    addr_to_vault_id.push((addr, vault_id));
                }
                Err(e) => {
                    tracing::warn!(%e, "Failed to list dynamic fields from peer vaults table");
                }
            }
        }

        if addr_to_vault_id.is_empty() {
            return Ok(HashMap::new());
        }

        let requests: Vec<GetObjectRequest> = addr_to_vault_id
            .iter()
            .map(|(_, vault_id)| GetObjectRequest::const_default().with_object_id(vault_id))
            .collect();

        let response = client
            .ledger_client()
            .batch_get_objects(
                BatchGetObjectsRequest::const_default()
                    .with_requests(requests)
                    .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["json"])),
            )
            .await
            .map_err(|e| ZingError::SuiClient(format!("batch_get_vaults: {}", e)))?;

        let results = response.into_inner().objects;

        let vaults: HashMap<String, PeerVaultInfo> = addr_to_vault_id
            .into_iter()
            .zip(results)
            .filter_map(|((addr, vault_id), result)| {
                let obj = result.result.and_then(|r| match r {
                    sui_rpc::proto::sui::rpc::v2::get_object_result::Result::Object(o) => Some(o),
                    _ => None,
                })?;
                let json = obj.json.as_ref()?;
                let info = parse_vault_json(json, &vault_id)?;
                Some((addr, info))
            })
            .collect();

        Ok(vaults)
    }

    pub async fn get_my_vault_info(&self) -> ZingResult<Option<PeerVaultInfo>> {
        let vaults = self.list_peer_vaults().await?;
        let addr_str = format!("{:#}", self.address);
        let addr_normalized = addr_str.trim_start_matches("0x").to_lowercase();
        Ok(vaults.into_iter().find_map(|(k, v)| {
            let k_normalized = k.trim_start_matches("0x").to_lowercase();
            if k_normalized == addr_normalized { Some(v) } else { None }
        }))
    }

    async fn get_vault_for_recipient(&self, recipient: Address) -> ZingResult<sui_sdk_types::Address> {
        let recipient_hex = format!("0x{}", hex::encode(<[u8; 32]>::from(recipient)));
        let vaults = self.list_peer_vaults().await?;
        if let Some(vault_info) = vaults.get(&recipient_hex) {
            return vault_info.vault_object_id.parse()
                .map_err(|e| ZingError::SuiClient(format!("parse vault id: {}", e)));
        }
        let settlement = self.settlement.as_ref()
            .ok_or_else(|| ZingError::SuiClient("settlement not configured".into()))?;
        settlement.vault_object_id
            .ok_or_else(|| ZingError::SuiClient("no vault_object_id in config".into()))
    }

    pub async fn create_vault(&self) -> ZingResult<()> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return Err(ZingError::SuiClient("settlement not configured".into())),
        };
        let tx = settlement.build_create_vault_transaction(self.address)
            .ok_or_else(|| ZingError::SuiClient("peer_vault_registry_id not set in config".into()))?;

        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let transaction = tx.build(&mut client).await
            .map_err(|e| ZingError::SuiClient(format!("build create_vault: {}", e)))?;

        let signature = self.keypair.sign_transaction(&transaction)
            .map_err(|e| ZingError::SuiClient(format!("sign: {}", e)))?;

        let request = sui_rpc::proto::sui::rpc::v2::ExecuteTransactionRequest::new(transaction.into())
            .with_signatures(vec![signature.into()]);

        let response = client
            .execute_transaction_and_wait_for_checkpoint(request, Duration::from_secs(60))
            .await
            .map_err(|e| ZingError::SuiClient(format!("create_vault tx: {}", e)))?;

        let digest = response.into_inner().transaction
            .and_then(|t| t.digest).unwrap_or_default();
        tracing::info!(tx_digest = %digest, "Vault created");
        Ok(())
    }

    pub async fn list_my_share_certificates(&self) -> ZingResult<Vec<ShareCertificateInfo>> {
        let settlement = match &self.settlement {
            Some(s) => s,
            None => return Ok(vec![]),
        };
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let owner = format!("{:#}", self.address);
        let cert_type = &settlement.share_certificate_type;

        // Pass 1: Get cert object IDs and versions via ListOwnedObjects
        use futures::TryStreamExt;
        let cert_ids: Vec<(String, u64)> = client
            .list_owned_objects(
                ListOwnedObjectsRequest::default()
                    .with_owner(&owner)
                    .with_object_type(cert_type.as_str())
                    .with_read_mask(FieldMask {
                        paths: vec!["object_id".into(), "version".into()],
                    }),
            )
            .try_filter_map(|obj| async move {
                Ok(Some((obj.object_id().to_string(), obj.version())))
            })
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| ZingError::SuiClient(format!("list owned: {}", e)))?;

        if cert_ids.is_empty() {
            return Ok(vec![]);
        }

        // Pass 2: Get JSON via batch_get_objects
        let requests: Vec<GetObjectRequest> = cert_ids
            .iter()
            .map(|(id, _)| GetObjectRequest::const_default().with_object_id(id))
            .collect();

        let response = client
            .ledger_client()
            .batch_get_objects(
                BatchGetObjectsRequest::const_default()
                    .with_requests(requests)
                    .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["json"])),
            )
            .await
            .map_err(|e| ZingError::SuiClient(format!("batch_get_certs: {}", e)))?;

        let results = response.into_inner().objects;

        // Parse each cert
        let certs: Vec<(String, u64, String, u64)> = cert_ids
            .into_iter()
            .zip(results)
            .filter_map(|((object_id, version), result)| {
                let obj = result.result.and_then(|r| match r {
                    sui_rpc::proto::sui::rpc::v2::get_object_result::Result::Object(o) => Some(o),
                    _ => None,
                })?;
                let json = obj.json.as_ref()?;
                let prost_types::value::Kind::StructValue(s) = json.kind.as_ref()? else { return None };
                let vault_id = s.fields.get("vault_id")
                    .and_then(|v| v.kind.as_ref())
                    .and_then(|k| match k { prost_types::value::Kind::StringValue(v) => Some(v.clone()), _ => None })?;
                let shares_str = s.fields.get("shares")
                    .and_then(|v| v.kind.as_ref())
                    .and_then(|k| match k { prost_types::value::Kind::StringValue(v) => Some(v.clone()), _ => None })?;
                let shares: u64 = shares_str.parse().ok()?;
                Some((object_id, version, vault_id, shares))
            })
            .collect();

        // Fetch vault info to map vault_id → peer address and compute estimated value
        let vaults = self.list_peer_vaults().await.unwrap_or_default();
        let vault_id_to_info: std::collections::HashMap<String, (String, PeerVaultInfo)> = vaults
            .into_iter()
            .map(|(addr, info)| (info.vault_object_id.clone(), (addr, info)))
            .collect();

        let result = certs
            .into_iter()
            .map(|(cert_object_id, cert_version, vault_id, shares)| {
                let vault_data = vault_id_to_info.get(&vault_id);
                let vault_address = vault_data.map(|(addr, _)| addr.clone()).unwrap_or_default();
                let estimated_value = vault_data.map(|(_, info)| {
                    if info.total_shares > 0 {
                        let product = (shares as u128).saturating_mul(info.reserves as u128);
                        (product / (info.total_shares as u128).max(1)) as u64
                    } else { 0 }
                }).unwrap_or(0);
                ShareCertificateInfo {
                    cert_object_id,
                    cert_version,
                    vault_id,
                    vault_address,
                    shares,
                    estimated_value,
                }
            })
            .collect();

        Ok(result)
    }

    pub async fn claim_earnings(&self) -> ZingResult<()> {
        let settlement = match &self.settlement {
            Some(s) => s,
            None => return Err(ZingError::SuiClient("settlement not configured".into())),
        };

        let vault_info = self.get_my_vault_info().await?
            .ok_or_else(|| ZingError::SuiClient("no vault created".into()))?;

        let vault_obj_id: sui_sdk_types::Address = vault_info.vault_object_id
            .parse()
            .map_err(|e| ZingError::SuiClient(format!("parse vault id: {}", e)))?;

        let tx = settlement.build_claim_earnings_transaction(
            self.address,
            vault_obj_id,
            settlement.vault_initial_shared_version,
        );

        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let transaction = tx.build(&mut client).await
            .map_err(|e| ZingError::SuiClient(format!("build claim_earnings: {}", e)))?;

        let signature = self.keypair.sign_transaction(&transaction)
            .map_err(|e| ZingError::SuiClient(format!("sign: {}", e)))?;

        let request = sui_rpc::proto::sui::rpc::v2::ExecuteTransactionRequest::new(transaction.into())
            .with_signatures(vec![signature.into()]);

        let response = client
            .execute_transaction_and_wait_for_checkpoint(request, Duration::from_secs(60))
            .await
            .map_err(|e| ZingError::SuiClient(format!("claim_earnings tx: {}", e)))?;

        let digest = response.into_inner().transaction
            .and_then(|t| t.digest).unwrap_or_default();
        tracing::info!(tx_digest = %digest, "Claim earnings executed");
        Ok(())
    }

    pub async fn delegate(&self, vault_object_id: &str, amount_frost: u64) -> ZingResult<()> {
        let settlement = match &self.settlement {
            Some(s) => s,
            None => return Err(ZingError::SuiClient("settlement not configured".into())),
        };

        let vault_obj_id: sui_sdk_types::Address = vault_object_id.parse()
            .map_err(|e| ZingError::SuiClient(format!("parse vault id: {}", e)))?;

        let tx = settlement.build_delegate_transaction(
            self.address,
            vault_obj_id,
            settlement.vault_initial_shared_version,
            amount_frost,
        );

        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let transaction = tx.build(&mut client).await
            .map_err(|e| ZingError::SuiClient(format!("build delegate: {}", e)))?;

        let signature = self.keypair.sign_transaction(&transaction)
            .map_err(|e| ZingError::SuiClient(format!("sign: {}", e)))?;

        let request = sui_rpc::proto::sui::rpc::v2::ExecuteTransactionRequest::new(transaction.into())
            .with_signatures(vec![signature.into()]);

        let response = client
            .execute_transaction_and_wait_for_checkpoint(request, Duration::from_secs(60))
            .await
            .map_err(|e| ZingError::SuiClient(format!("delegate tx: {}", e)))?;

        let digest = response.into_inner().transaction
            .and_then(|t| t.digest).unwrap_or_default();
        tracing::info!(tx_digest = %digest, vault = %vault_object_id, amount = amount_frost, "Delegate executed");
        Ok(())
    }

    pub async fn undelegate(&self, cert_object_id: &str) -> ZingResult<()> {
        let settlement = match &self.settlement {
            Some(s) => s,
            None => return Err(ZingError::SuiClient("settlement not configured".into())),
        };
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        // Fetch the cert object to get vault_id and current version
        let response = client.ledger_client().get_object(
            GetObjectRequest::const_default()
                .with_object_id(cert_object_id)
                .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["json", "version", "digest"])),
        ).await
        .map_err(|e| ZingError::SuiClient(format!("get cert: {}", e)))?;

        let obj = response.into_inner().object
            .ok_or_else(|| ZingError::SuiClient("cert object not found".into()))?;

        let cert_version = obj.version
            .ok_or_else(|| ZingError::SuiClient("cert has no version".into()))?;

        let cert_digest: sui_sdk_types::Digest = obj.digest
            .ok_or_else(|| ZingError::SuiClient("cert has no digest".into()))?
            .parse()
            .map_err(|e| ZingError::SuiClient(format!("parse cert digest: {}", e)))?;

        let json = obj.json.as_ref()
            .ok_or_else(|| ZingError::SuiClient("cert has no json".into()))?;

        let vault_id = {
            let prost_types::value::Kind::StructValue(s) = json.kind.as_ref()
                .ok_or_else(|| ZingError::SuiClient("cert json not a struct".into()))? else { unreachable!() };
            s.fields.get("vault_id")
                .and_then(|v| v.kind.as_ref())
                .and_then(|k| match k { prost_types::value::Kind::StringValue(v) => Some(v.clone()), _ => None })
                .ok_or_else(|| ZingError::SuiClient("missing vault_id in cert".into()))?
        };

        let vault_obj_id: sui_sdk_types::Address = vault_id.parse()
            .map_err(|e| ZingError::SuiClient(format!("parse vault id: {}", e)))?;
        let cert_obj_id: sui_sdk_types::Address = cert_object_id.parse()
            .map_err(|e| ZingError::SuiClient(format!("parse cert id: {}", e)))?;

        let tx = settlement.build_undelegate_transaction(
            self.address,
            vault_obj_id,
            settlement.vault_initial_shared_version,
            cert_obj_id,
            cert_version,
            cert_digest,
        );

        let transaction = tx.build(&mut client).await
            .map_err(|e| ZingError::SuiClient(format!("build undelegate: {}", e)))?;

        let signature = self.keypair.sign_transaction(&transaction)
            .map_err(|e| ZingError::SuiClient(format!("sign: {}", e)))?;

        let request = sui_rpc::proto::sui::rpc::v2::ExecuteTransactionRequest::new(transaction.into())
            .with_signatures(vec![signature.into()]);

        let response = client
            .execute_transaction_and_wait_for_checkpoint(request, Duration::from_secs(60))
            .await
            .map_err(|e| ZingError::SuiClient(format!("undelegate tx: {}", e)))?;

        let digest = response.into_inner().transaction
            .and_then(|t| t.digest).unwrap_or_default();
        tracing::info!(tx_digest = %digest, cert = %cert_object_id, "Undelegate executed");
        Ok(())
    }

    pub fn settlement_config(&self) -> Option<&SettlementConfig> { self.settlement.as_ref() }

    /// Fetch the on-chain registered Peer info for this wallet.
    /// Returns None if not registered.
    pub async fn get_registered_peer_info(&self) -> ZingResult<Option<RegisteredPeerInfo>> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return Ok(None),
        };
        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let peers_table_id = format!("0x{}", hex::encode(settlement.registry_peers_table_id));
        let address_bytes: [u8; 32] = self.address.into();

        let request = ListDynamicFieldsRequest::const_default()
            .with_parent(&peers_table_id)
            .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["name", "value"]));

        let stream = client.list_dynamic_fields(request);
        let mut stream = std::pin::pin!(stream);

        let mut peer_object_id: Option<String> = None;

        while let Some(result) = stream.next().await {
            match result {
                Ok(df) => {
                    if let Some(name_bcs) = &df.name {
                        if let Some(name_bytes) = &name_bcs.value {
                            if name_bytes.as_ref() == address_bytes {
                                let obj_id = if let Some(child_id) = &df.child_id {
                                    child_id.clone()
                                } else if let Some(value_bcs) = &df.value {
                                    if let Some(value_bytes) = &value_bcs.value {
                                        format!("0x{}", hex::encode(value_bytes.as_ref()))
                                    } else { continue; }
                                } else { continue; };
                                peer_object_id = Some(obj_id);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(%e, "Failed to list dynamic fields from peers table");
                }
            }
        }

        let peer_object_id = match peer_object_id {
            Some(id) => id,
            None => return Ok(None),
        };

        let response = client.ledger_client().get_object(
            GetObjectRequest::const_default()
                .with_object_id(&peer_object_id)
                .with_read_mask(sui_rpc::field::FieldMask::from_paths(vec!["json", "version", "owner"])),
        ).await
        .map_err(|e| ZingError::SuiClient(format!("get peer object: {}", e)))?;

        let object = response.into_inner().object
            .ok_or_else(|| ZingError::SuiClient("peer object not found".into()))?;

        let json = object.json.as_ref()
            .ok_or_else(|| ZingError::SuiClient("peer object has no json".into()))?;

        let peer_id_bytes = parse_peer_id_from_json(json)?;

        let parse_debug = PeerId::from_bytes(&peer_id_bytes)
            .map(|p| p.to_string())
            .unwrap_or_else(|_| format!("raw({}b)", peer_id_bytes.len()));
        tracing::info!(
            peer_obj_id = %peer_object_id,
            parsed_peer_id = %parse_debug,
            bytes_len = peer_id_bytes.len(),
            "get_registered_peer_info: parsed peer object"
        );

        let initial_shared_version = object.owner
            .as_ref()
            .and_then(|o| o.version)
            .ok_or_else(|| ZingError::SuiClient("peer object has no initial_shared_version".into()))?;

        Ok(Some(RegisteredPeerInfo {
            peer_id_bytes,
            peer_object_id,
            initial_shared_version,
        }))
    }

    /// Update the registered peer_id on-chain if it differs from the current peer_id.
    pub async fn update_peer_id(&self, new_peer_id_bytes: Vec<u8>) -> ZingResult<()> {
        let settlement = match &self.settlement {
            Some(s) => s, None => return Ok(()),
        };

        let info = self.get_registered_peer_info().await?
            .ok_or_else(|| ZingError::SuiClient("not registered".into()))?;

        let current_pid = PeerId::from_bytes(&new_peer_id_bytes)
            .map(|p| p.to_string())
            .unwrap_or_else(|_| format!("raw({}b)", new_peer_id_bytes.len()));
        let onchain_pid = PeerId::from_bytes(&info.peer_id_bytes)
            .map(|p| p.to_string())
            .unwrap_or_else(|_| format!("raw({}b)", info.peer_id_bytes.len()));

        tracing::info!(
            "Checking peer ID: current={current_pid} onchain={onchain_pid} match={}",
            info.peer_id_bytes == new_peer_id_bytes
        );

        if info.peer_id_bytes == new_peer_id_bytes {
            tracing::info!("Peer ID already matches on-chain, skipping update");
            return Ok(());
        }
        tracing::info!("Peer ID mismatch detected, updating on-chain");

        let mut client = sui_rpc::Client::new(&self.rpc_url)
            .map_err(|e| ZingError::SuiClient(format!("rpc: {}", e)))?;

        let peer_obj_addr: sui_sdk_types::Address = info.peer_object_id.parse()
            .map_err(|e| ZingError::SuiClient(format!("parse peer object id: {}", e)))?;

        let tx = settlement.build_update_peer_id_transaction(
            self.address, peer_obj_addr, info.initial_shared_version, new_peer_id_bytes,
        );

        let transaction = tx.build(&mut client).await
            .map_err(|e| ZingError::SuiClient(format!("build update: {}", e)))?;

        let signature = self.keypair.sign_transaction(&transaction)
            .map_err(|e| ZingError::SuiClient(format!("sign: {}", e)))?;

        let request = sui_rpc::proto::sui::rpc::v2::ExecuteTransactionRequest::new(transaction.into())
            .with_signatures(vec![signature.into()]);

        let response = client
            .execute_transaction_and_wait_for_checkpoint(request, Duration::from_secs(60))
            .await
            .map_err(|e| ZingError::SuiClient(format!("update peer id tx: {}", e)))?;

        let digest = response.into_inner().transaction
            .and_then(|t| t.digest).unwrap_or_default();
        tracing::info!(tx_digest = %digest, "Peer ID updated on-chain");
        Ok(())
    }

    /// Ensure the on-chain peer_id matches the current peer_id.
    /// Registers if not registered, updates if mismatch.
    pub async fn ensure_peer_registered(&self, peer_id_bytes: Vec<u8>) -> ZingResult<()> {
        match self.is_peer_registered().await {
            Ok(true) => {
                self.update_peer_id(peer_id_bytes).await
            }
            _ => {
                self.register_peer(peer_id_bytes).await
            }
        }
    }

    pub fn to_libp2p_keypair(&self) -> libp2p::identity::Keypair {
        let mut seed = self.seed;
        libp2p::identity::Keypair::ed25519_from_bytes(&mut seed[..])
            .expect("valid ed25519 seed from wallet")
    }

    fn synthetic_pay(
        &self, recipient: Address, blob_hash: &[u8; 32], amount: u64,
    ) -> ZingResult<PaymentProof> {
        let c = self.payment_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let mut h = Sha256::new();
        h.update(b"zing-payment-v1");
        h.update(recipient.to_string().as_bytes());
        h.update(blob_hash);
        h.update(amount.to_le_bytes());
        h.update(c.to_le_bytes());
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
            seed: [0u8; 32],
            settlement: None,
            rpc_url: String::new(),
            payment_counter: AtomicU64::new(0),
        }
    }
}

fn decode_and_derive(keys: Vec<String>) -> ZingResult<(Ed25519PrivateKey, Address, [u8; 32])> {
    for k in &keys {
        let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, k)
            .map_err(|e| ZingError::SuiClient(format!("key decode: {}", e)))?;
        if raw.len() == 33 && raw[0] == 0x00 {
            let seed: [u8; 32] = raw[1..].try_into()
                .map_err(|_| ZingError::SuiClient("invalid key length".into()))?;
            let kp = Ed25519PrivateKey::new(seed);
            let addr = kp.public_key().derive_address();
            return Ok((kp, addr, seed));
        }
    }
    Err(ZingError::SuiClient("no Ed25519 key found".into()))
}

fn decode_first_matching(keys: Vec<String>, target: Address) -> ZingResult<(Ed25519PrivateKey, Address, [u8; 32])> {
    for k in &keys {
        let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, k)
            .map_err(|e| ZingError::SuiClient(format!("key decode: {}", e)))?;
        if raw.len() == 33 && raw[0] == 0x00 {
            let seed: [u8; 32] = raw[1..].try_into()
                .map_err(|_| ZingError::SuiClient("invalid key length".into()))?;
            let kp = Ed25519PrivateKey::new(seed);
            let addr = kp.public_key().derive_address();
            if addr == target {
                return Ok((kp, addr, seed));
            }
        }
    }
    Err(ZingError::SuiClient("no Ed25519 key matching active_address".into()))
}

fn parse_active_address(client_yaml: &Path) -> Option<Address> {
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

fn parse_peer_id_from_json(json: &prost_types::Value) -> ZingResult<Vec<u8>> {
    let s = match json.kind.as_ref() {
        Some(prost_types::value::Kind::StructValue(s)) => s,
        _ => return Err(ZingError::SuiClient("peer object json not a struct".into())),
    };

    let peer_id_b64 = s.fields.get("peer_id")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        }).ok_or_else(|| ZingError::SuiClient("missing peer_id field".into()))?;

    base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &peer_id_b64,
    ).map_err(|e| ZingError::SuiClient(format!("decode peer_id: {}", e)))
}

fn parse_vault_json(json: &prost_types::Value, vault_object_id: &str) -> Option<PeerVaultInfo> {
    let prost_types::value::Kind::StructValue(s) = json.kind.as_ref()? else { return None; };

    let reserves_str = s.fields.get("reserves")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        })?;
    let reserves: u64 = reserves_str.parse().ok()?;

    let total_shares_str = s.fields.get("total_shares")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        })?;
    let total_shares: u64 = total_shares_str.parse().ok()?;

    let commission_bps_str = s.fields.get("commission_bps")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        })?;
    let commission_bps: u64 = commission_bps_str.parse().ok()?;

    let peer_earnings_str = s.fields.get("peer_earnings")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        })?;
    let peer_earnings: u64 = peer_earnings_str.parse().ok()?;

    Some(PeerVaultInfo { reserves, total_shares, commission_bps, peer_earnings, vault_object_id: vault_object_id.to_string() })
}

fn parse_peer_json(json: &prost_types::Value, obj_id: &str) -> Option<PeerInfo> {
    let prost_types::value::Kind::StructValue(s) = json.kind.as_ref()? else { return None; };

    let peer_id_b64 = s.fields.get("peer_id")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        })?;

    let peer_id_bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &peer_id_b64,
    ).ok()?;

    let peer_id_b58 = PeerId::from_bytes(&peer_id_bytes).ok()?.to_string();

    let sui_address = s.fields.get("sui_address")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        })?;

    let bond = s.fields.get("bond")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => s.parse().ok(),
            _ => None,
        })?;

    let is_active = s.fields.get("is_active")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::BoolValue(b) => Some(*b),
            _ => None,
        })?;

    Some(PeerInfo { sui_address, peer_id_b58, bond, is_active, peer_object_id: obj_id.to_string(), vault: None })
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
    #[tokio::test]
    #[ignore] // Requires network, run with: cargo test -- --ignored
    async fn test_is_peer_registered_on_mainnet() {
        let vault_id: sui_sdk_types::Address =
            "0x16e909500ee62ea4acf2a0cc9b5fcff86e27e7aa38d39dfc32de6bd73cfca431"
                .parse().unwrap();
        let settlement = SettlementConfig::mainnet(vault_id);
        let wallet = ZingWallet {
            address: "0x0b3fc768f8bb3c772321e3e7781cac4a45585b4bc64043686beb634d65341798"
                .parse().unwrap(),
            keypair: Ed25519PrivateKey::new([0u8; 32]),
            seed: [0u8; 32],
            settlement: Some(settlement),
            rpc_url: "https://fullnode.mainnet.sui.io:443".into(),
            payment_counter: AtomicU64::new(0),
        };

        let result = wallet.is_peer_registered().await.unwrap();
        assert!(result, "Peer should be registered on mainnet");
    }
}
