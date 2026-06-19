/// Configuration for the on-chain settlement contract.
#[derive(Debug, Clone)]
pub struct SettlementConfig {
    pub package_id: sui_sdk_types::Address,
    pub settlement_object_id: sui_sdk_types::Address,
    pub registry_object_id: sui_sdk_types::Address,
    pub vault_object_id: Option<sui_sdk_types::Address>,
    pub wal_coin_type: String,
    pub wal_package_id: sui_sdk_types::Address,
    pub registry_initial_shared_version: u64,
    pub settlement_initial_shared_version: u64,
    pub vault_initial_shared_version: u64,
}

impl SettlementConfig {
    pub fn mainnet(vault_object_id: sui_sdk_types::Address) -> Self {
        Self {
            package_id: "0xc584ff1d0d76f4da6aa3b9115263f248e1b0cf60b37d0fc96d2b49b2b72997c8"
                .parse().expect("invalid package_id"),
            settlement_object_id: "0xc58e9b7417fdc83743b46a3f9009b10868f05bb1f2283f08c7021ac3e7f6c308"
                .parse().expect("invalid settlement_object_id"),
            registry_object_id: "0x97b5153b9e9897ad1630cdd06e5caa81ebbf8865e96003f38e50c5f1d6752527"
                .parse().expect("invalid registry_object_id"),
            vault_object_id: Some(vault_object_id),
            wal_coin_type: "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL".into(),
            wal_package_id: "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59"
                .parse().expect("invalid wal_package_id"),
            registry_initial_shared_version: 921074118,
            settlement_initial_shared_version: 921074118,
            vault_initial_shared_version: 921074119,
        }
    }

    fn wal_struct_tag(&self) -> sui_sdk_types::StructTag {
        use std::str::FromStr;
        sui_sdk_types::StructTag::from_str(&self.wal_coin_type)
            .expect("invalid wal_coin_type in config")
    }

    /// Build PTB for settlement::pay(). Uses tx.coin() to split exact payment amount.
    pub fn build_pay_transaction(
        &self,
        sender: sui_sdk_types::Address,
        recipient: sui_sdk_types::Address,
        blob_hash: &[u8; 32],
        amount: u64,
    ) -> sui_transaction_builder::TransactionBuilder {
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

        let vault_obj_id = self.vault_object_id.expect("vault_object_id not set");

        // Coin intent — auto-selects + splits exact amount
        let payment_arg = tx.intent(
            sui_transaction_builder::intent::CoinWithBalance::new(self.wal_struct_tag(), amount)
        );

        let settlement_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            self.settlement_object_id, self.settlement_initial_shared_version, false,
        ));
        let vault_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            vault_obj_id, self.vault_initial_shared_version, true,
        ));
        let payee_input = tx.pure(&recipient);
        let blob_hash_input = { let v = blob_hash.to_vec(); tx.pure(&v) };

        tx.move_call(
            sui_transaction_builder::Function::new(
                self.package_id,
                sui_sdk_types::Identifier::from_static("settlement"),
                sui_sdk_types::Identifier::from_static("pay"),
            ),
            vec![settlement_input, vault_input, payee_input, blob_hash_input, payment_arg],
        );

        tx.set_sender(sender);
        tx.set_gas_budget(5_000_000);
        tx
    }

    /// Build PTB for staking::register(). Uses tx.coin() for bond amount.
    pub fn build_register_transaction(
        &self,
        sender: sui_sdk_types::Address,
        peer_id_bytes: Vec<u8>,
        amount: u64,
    ) -> sui_transaction_builder::TransactionBuilder {
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

        let bond_arg = tx.intent(
            sui_transaction_builder::intent::CoinWithBalance::new(self.wal_struct_tag(), amount)
        );

        let registry_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            self.registry_object_id, self.registry_initial_shared_version, true,
        ));
        let peer_id_input = tx.pure(&peer_id_bytes);

        tx.move_call(
            sui_transaction_builder::Function::new(
                self.package_id,
                sui_sdk_types::Identifier::from_static("staking"),
                sui_sdk_types::Identifier::from_static("register"),
            ),
            vec![registry_input, peer_id_input, bond_arg],
        );

        tx.set_sender(sender);
        tx.set_gas_budget(10_000_000);
        tx
    }
}
