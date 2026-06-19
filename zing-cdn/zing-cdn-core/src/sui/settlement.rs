/// Configuration for the on-chain settlement contract.
/// Stores the deployed package and shared object IDs on Sui mainnet.
#[derive(Debug, Clone)]
pub struct SettlementConfig {
    /// Package ID of the deployed zing_cdn package
    pub package_id: sui_sdk_types::Address,
    /// Object ID of the shared Settlement object
    pub settlement_object_id: sui_sdk_types::Address,
    /// Object ID of the shared Registry object (for auto-registration)
    pub registry_object_id: sui_sdk_types::Address,
    /// Object ID of the serving peer's PeerVault (per-fetch)
    pub vault_object_id: Option<sui_sdk_types::Address>,
    /// WAL coin type: "0x356a...::wal::WAL"
    pub wal_coin_type: String,
    /// WAL package ID (for coin type in PTB)
    pub wal_package_id: sui_sdk_types::Address,
    /// Initial shared version of Registry (query from suiscan or RPC)
    pub registry_initial_shared_version: u64,
    /// Initial shared version of Settlement
    pub settlement_initial_shared_version: u64,
    /// Initial shared version of PeerVault
    pub vault_initial_shared_version: u64,
}

impl SettlementConfig {
    /// Creates a SettlementConfig with mainnet addresses.
    pub fn mainnet(vault_object_id: sui_sdk_types::Address) -> Self {
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

    /// Builds a PTB that calls settlement::pay().
    pub fn build_pay_transaction(
        &self,
        sender: sui_sdk_types::Address,
        recipient: sui_sdk_types::Address,
        blob_hash: &[u8; 32],
        payment_coin: (sui_sdk_types::Address, u64, sui_sdk_types::Digest),
        gas_budget: u64,
    ) -> sui_transaction_builder::TransactionBuilder {
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

        let vault_obj_id = self.vault_object_id
            .expect("vault_object_id not set");

        // Payment coin input
        let payment_input = tx.object(sui_transaction_builder::ObjectInput::owned(
            payment_coin.0, payment_coin.1, payment_coin.2,
        ));

        // Settlement shared object (immutable)
        let settlement_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            self.settlement_object_id,
            self.settlement_initial_shared_version,
            false,
        ));

        // Vault shared object (mutable)
        let vault_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            vault_obj_id,
            self.vault_initial_shared_version,
            true,
        ));

        // Pure inputs — Address and Vec<u8> are auto-encoded correctly
        let payee_input = tx.pure(&recipient);
        let blob_hash_input = {let v = blob_hash.to_vec(); tx.pure(&v)};

        // settlement::pay(Settlement, Vault, payee, blob_hash, coin)
        tx.move_call(
            sui_transaction_builder::Function::new(
                self.package_id,
                sui_sdk_types::Identifier::from_static("settlement"),
                sui_sdk_types::Identifier::from_static("pay"),
            ),
            vec![settlement_input, vault_input, payee_input, blob_hash_input, payment_input],
        );

        tx.set_sender(sender);
        tx.set_gas_budget(gas_budget);
        tx
    }

    /// Builds a PTB that calls staking::register() for auto-registration.
    pub fn build_register_transaction(
        &self,
        sender: sui_sdk_types::Address,
        peer_id_bytes: Vec<u8>,
        bond_coin: (sui_sdk_types::Address, u64, sui_sdk_types::Digest),
        gas_budget: u64,
    ) -> sui_transaction_builder::TransactionBuilder {
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

        let bond_input = tx.object(sui_transaction_builder::ObjectInput::owned(
            bond_coin.0, bond_coin.1, bond_coin.2,
        ));

        let registry_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            self.registry_object_id,
            self.registry_initial_shared_version,
            true,
        ));

        let peer_id_input = tx.pure(&peer_id_bytes);

        tx.move_call(
            sui_transaction_builder::Function::new(
                self.package_id,
                sui_sdk_types::Identifier::from_static("staking"),
                sui_sdk_types::Identifier::from_static("register"),
            ),
            vec![registry_input, peer_id_input, bond_input],
        );

        tx.set_sender(sender);
        tx.set_gas_budget(gas_budget);
        tx
    }
}
