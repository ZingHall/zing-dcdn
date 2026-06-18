use sui_sdk::types::base_types::{ObjectID, SuiAddress};

/// Configuration for the on-chain settlement contract.
/// Stores the deployed package and shared object IDs on Sui mainnet.
#[derive(Debug, Clone)]
pub struct SettlementConfig {
    /// Package ID of the deployed zing_cdn package
    pub package_id: ObjectID,
    /// Object ID of the shared Settlement object
    pub settlement_object_id: ObjectID,
    /// Object ID of the serving peer's PeerVault (per-fetch)
    pub vault_object_id: Option<ObjectID>,
    /// WAL coin type: "0x356a...::wal::WAL"
    pub wal_coin_type: String,
    /// WAL package ID (for coin type in PTB)
    pub wal_package_id: ObjectID,
}

impl SettlementConfig {
    /// Builds a ProgrammableTransaction that calls settlement::pay().
    ///
    /// The PTB:
    ///   1. Takes the WAL payment coin as input
    ///   2. Calls settlement::pay(settlement_obj, vault_obj, payee, blob_hash, coin)
    ///
    /// Returns the unsigned TransactionData, which must be signed and submitted.
    ///
    /// # Arguments
    /// * `recipient` — Sui address of the serving peer (payee)
    /// * `blob_hash` — 32-byte blob identifier
    /// * `payment_coin` — object reference to the WAL coin to spend
    /// * `gas_coin` — object reference for gas payment
    /// * `gas_budget` — max gas in MIST
    pub fn build_pay_transaction(
        &self,
        sender: SuiAddress,
        recipient: SuiAddress,
        blob_hash: &[u8; 32],
        payment_coin: sui_sdk::types::base_types::ObjectRef,
        gas_coin: sui_sdk::types::base_types::ObjectRef,
        gas_budget: u64,
        gas_price: u64,
    ) -> Result<sui_sdk::types::transaction::TransactionData, anyhow::Error> {
        use sui_sdk::types::{
            programmable_transaction_builder::ProgrammableTransactionBuilder,
            transaction::{TransactionData, TransactionDataV1, ObjectArg, SharedObjectMutability},
        };

        let vault_obj_id = self
            .vault_object_id
            .ok_or_else(|| anyhow::anyhow!("vault_object_id not set"))?;

        let mut ptb = ProgrammableTransactionBuilder::new();

        // Input the payment coin (WAL)
        let payment_input = ptb.input(
            sui_sdk::types::transaction::CallArg::Object(ObjectArg::ImmOrOwnedObject(
                payment_coin,
            )),
        )
        .map_err(|e| anyhow::anyhow!("ptb input error: {}", e))?;

        // Input the Settlement shared object
        let settlement_input = ptb.input(
            sui_sdk::types::transaction::CallArg::Object(ObjectArg::SharedObject {
                id: self.settlement_object_id,
                initial_shared_version: sui_sdk::types::base_types::SequenceNumber::from_u64(1),
                mutability: SharedObjectMutability::Immutable,
            }),
        )
        .map_err(|e| anyhow::anyhow!("ptb input error: {}", e))?;

        // Input the Vault shared object (mutable)
        let vault_input = ptb.input(
            sui_sdk::types::transaction::CallArg::Object(ObjectArg::SharedObject {
                id: vault_obj_id,
                initial_shared_version: sui_sdk::types::base_types::SequenceNumber::from_u64(1),
                mutability: SharedObjectMutability::Mutable,
            }),
        )
        .map_err(|e| anyhow::anyhow!("ptb input error: {}", e))?;

        // Pure inputs
        let payee_input = ptb.pure(recipient.to_vec())
            .map_err(|e| anyhow::anyhow!("ptb pure error: {}", e))?;
        let blob_hash_input = ptb.pure(blob_hash.to_vec())
            .map_err(|e| anyhow::anyhow!("ptb pure error: {}", e))?;

        // Command: settlement::pay(settlement, vault, payee, blob_hash, coin)
        let module = sui_sdk::types::Identifier::new("settlement")
            .map_err(|e| anyhow::anyhow!("invalid module name: {}", e))?;
        let function = sui_sdk::types::Identifier::new("pay")
            .map_err(|e| anyhow::anyhow!("invalid function name: {}", e))?;
        ptb.command(sui_sdk::types::transaction::Command::move_call(
            self.package_id,
            module,
            function,
            vec![], // no type args
            vec![settlement_input, vault_input, payee_input, blob_hash_input, payment_input],
        ));

        let pt = ptb.finish();

        Ok(TransactionData::V1(TransactionDataV1 {
            kind: sui_sdk::types::transaction::TransactionKind::ProgrammableTransaction(pt),
            sender,
            gas_data: sui_sdk::types::transaction::GasData {
                payment: vec![gas_coin],
                owner: sender,
                price: gas_price,
                budget: gas_budget,
            },
            expiration: sui_sdk::types::transaction::TransactionExpiration::None,
        }))
    }
}
