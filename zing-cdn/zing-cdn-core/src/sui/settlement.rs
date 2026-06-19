use sui_sdk::types::base_types::{ObjectID, SuiAddress};

/// Configuration for the on-chain settlement contract.
/// Stores the deployed package and shared object IDs on Sui mainnet.
#[derive(Debug, Clone)]
pub struct SettlementConfig {
    /// Package ID of the deployed zing_cdn package
    pub package_id: ObjectID,
    /// Object ID of the shared Settlement object
    pub settlement_object_id: ObjectID,
    /// Object ID of the shared Registry object (for auto-registration)
    pub registry_object_id: ObjectID,
    /// Object ID of the serving peer's PeerVault (per-fetch)
    pub vault_object_id: Option<ObjectID>,
    /// WAL coin type: "0x356a...::wal::WAL"
    pub wal_coin_type: String,
    /// WAL package ID (for coin type in PTB)
    pub wal_package_id: ObjectID,
    /// Initial shared version of Registry (query from suiscan or RPC)
    pub registry_initial_shared_version: u64,
    /// Initial shared version of Settlement
    pub settlement_initial_shared_version: u64,
    /// Initial shared version of PeerVault
    pub vault_initial_shared_version: u64,
}

impl SettlementConfig {
    /// Creates a SettlementConfig with mainnet addresses.
    pub fn mainnet(vault_object_id: ObjectID) -> Self {
        Self {
            package_id: "0xc584ff1d0d76f4da6aa3b9115263f248e1b0cf60b37d0fc96d2b49b2b72997c8"
                .parse()
                .expect("invalid package_id"),
            settlement_object_id: "0xc58e9b7417fdc83743b46a3f9009b10868f05bb1f2283f08c7021ac3e7f6c308"
                .parse()
                .expect("invalid settlement_object_id"),
            registry_object_id: "0x97b5153b9e9897ad1630cdd06e5caa81ebbf8865e96003f38e50c5f1d6752527"
                .parse()
                .expect("invalid registry_object_id"),
            vault_object_id: Some(vault_object_id),
            wal_coin_type: "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL".into(),
            wal_package_id: "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59"
                .parse()
                .expect("invalid wal_package_id"),
            registry_initial_shared_version: 921074118,
            settlement_initial_shared_version: 921074118,
            vault_initial_shared_version: 921074119,
        }
    }

    /// Builds a ProgrammableTransaction that calls settlement::pay().
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

        let payment_input = ptb.input(
            sui_sdk::types::transaction::CallArg::Object(ObjectArg::ImmOrOwnedObject(
                payment_coin,
            )),
        )
        .map_err(|e| anyhow::anyhow!("ptb input error: {}", e))?;

        let settlement_input = ptb.input(
            sui_sdk::types::transaction::CallArg::Object(ObjectArg::SharedObject {
                id: self.settlement_object_id,
                initial_shared_version: sui_sdk::types::base_types::SequenceNumber::from_u64(self.settlement_initial_shared_version),
                mutability: SharedObjectMutability::Immutable,
            }),
        )
        .map_err(|e| anyhow::anyhow!("ptb input error: {}", e))?;

        let vault_input = ptb.input(
            sui_sdk::types::transaction::CallArg::Object(ObjectArg::SharedObject {
                id: vault_obj_id,
                initial_shared_version: sui_sdk::types::base_types::SequenceNumber::from_u64(self.vault_initial_shared_version),
                mutability: SharedObjectMutability::Mutable,
            }),
        )
        .map_err(|e| anyhow::anyhow!("ptb input error: {}", e))?;

        let payee_input = ptb.pure(recipient.to_vec())
            .map_err(|e| anyhow::anyhow!("ptb pure error: {}", e))?;
        let blob_hash_input = ptb.pure(blob_hash.to_vec())
            .map_err(|e| anyhow::anyhow!("ptb pure error: {}", e))?;

        let module = sui_sdk::types::Identifier::new("settlement")
            .map_err(|e| anyhow::anyhow!("invalid module name: {}", e))?;
        let function = sui_sdk::types::Identifier::new("pay")
            .map_err(|e| anyhow::anyhow!("invalid function name: {}", e))?;
        ptb.command(sui_sdk::types::transaction::Command::move_call(
            self.package_id,
            module,
            function,
            vec![],
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

    /// Builds a PTB that calls staking::register() for auto-registration.
    pub fn build_register_transaction(
        &self,
        sender: SuiAddress,
        peer_id_bytes: Vec<u8>,
        bond_coin: sui_sdk::types::base_types::ObjectRef,
        gas_coin: sui_sdk::types::base_types::ObjectRef,
        gas_budget: u64,
        gas_price: u64,
    ) -> Result<sui_sdk::types::transaction::TransactionData, anyhow::Error> {
        use sui_sdk::types::{
            programmable_transaction_builder::ProgrammableTransactionBuilder,
            transaction::{TransactionData, TransactionDataV1, ObjectArg, SharedObjectMutability},
        };

        let mut ptb = ProgrammableTransactionBuilder::new();

        let bond_input = ptb.input(
            sui_sdk::types::transaction::CallArg::Object(ObjectArg::ImmOrOwnedObject(
                bond_coin,
            )),
        )
        .map_err(|e| anyhow::anyhow!("ptb input error: {}", e))?;

        let registry_input = ptb.input(
            sui_sdk::types::transaction::CallArg::Object(ObjectArg::SharedObject {
                id: self.registry_object_id,
                initial_shared_version: sui_sdk::types::base_types::SequenceNumber::from_u64(self.registry_initial_shared_version),
                mutability: SharedObjectMutability::Mutable,
            }),
        )
        .map_err(|e| anyhow::anyhow!("ptb input error: {}", e))?;

        let peer_id_input = ptb.pure(peer_id_bytes)
            .map_err(|e| anyhow::anyhow!("ptb pure error: {}", e))?;

        let module = sui_sdk::types::Identifier::new("staking")
            .map_err(|e| anyhow::anyhow!("invalid module name: {}", e))?;
        let function = sui_sdk::types::Identifier::new("register")
            .map_err(|e| anyhow::anyhow!("invalid function name: {}", e))?;
        ptb.command(sui_sdk::types::transaction::Command::move_call(
            self.package_id,
            module,
            function,
            vec![],
            vec![registry_input, peer_id_input, bond_input],
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
