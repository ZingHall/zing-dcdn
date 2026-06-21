/// Configuration for the on-chain settlement contract.
#[derive(Debug, Clone)]
pub struct SettlementConfig {
    pub package_id: sui_sdk_types::Address,
    pub settlement_object_id: sui_sdk_types::Address,
    pub registry_object_id: sui_sdk_types::Address,
    pub registry_peers_table_id: sui_sdk_types::Address,
    pub peer_vaults_table_id: [u8; 32],
    pub peer_vault_registry_id: Option<sui_sdk_types::Address>,
    pub vault_object_id: Option<sui_sdk_types::Address>,
    pub wal_coin_type: String,
    pub wal_package_id: sui_sdk_types::Address,
    pub registry_initial_shared_version: u64,
    pub settlement_initial_shared_version: u64,
    pub vault_initial_shared_version: u64,
    pub peer_vaults_initial_shared_version: u64,
    pub peer_vault_registry_initial_shared_version: u64,
    pub share_certificate_type: String,
}

impl SettlementConfig {
    pub fn mainnet(vault_object_id: sui_sdk_types::Address) -> Self {
        let peer_vaults_table_str = "0x465bf3e99dff79a56705b111396ee5b9bd35f2a1aac70d118f466a7c581e0e07";
        let peer_vaults_table_stripped = peer_vaults_table_str.strip_prefix("0x").unwrap_or(peer_vaults_table_str);
        let mut peer_vaults_table_id = [0u8; 32];
        hex::decode_to_slice(peer_vaults_table_stripped, &mut peer_vaults_table_id)
            .expect("invalid peer_vaults_table_id");
        Self {
            package_id: "0xb4307939d0cf205880746372d8a467af67f886122fde3ed69fd912885848e8f8"
                .parse().expect("invalid package_id"),
            settlement_object_id: "0xc58e9b7417fdc83743b46a3f9009b10868f05bb1f2283f08c7021ac3e7f6c308"
                .parse().expect("invalid settlement_object_id"),
            registry_object_id: "0x97b5153b9e9897ad1630cdd06e5caa81ebbf8865e96003f38e50c5f1d6752527"
                .parse().expect("invalid registry_object_id"),
            registry_peers_table_id: "0xbcd17d4df8489569fdca7bc9a795c16a73560efbde2355d91ef9195bf676ea00"
                .parse().expect("invalid registry_peers_table_id"),
            peer_vaults_table_id,
            peer_vault_registry_id: Some("0x9b96aa341bc3749283f9320ae783f2e6aff86b6393a45aeeedc53946f089d615"
                .parse().expect("invalid peer_vault_registry_id")),
            vault_object_id: Some(vault_object_id),
            wal_coin_type: "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL".into(),
            wal_package_id: "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59"
                .parse().expect("invalid wal_package_id"),
            registry_initial_shared_version: 921074118,
            settlement_initial_shared_version: 921074118,
            vault_initial_shared_version: 921074119,
            peer_vaults_initial_shared_version: 923306507,
            peer_vault_registry_initial_shared_version: 923306507,
            share_certificate_type: "0x9dd1a5dc551e322dd1b0394514ece30eb1e5f54d5de5b1f6fe135ebe24032b9c::peer_vault::ShareCertificate".into(),
        }
    }

    fn wal_struct_tag(&self) -> sui_sdk_types::StructTag {
        use std::str::FromStr;
        sui_sdk_types::StructTag::from_str(&self.wal_coin_type)
            .expect("invalid wal_coin_type in config")
    }

    /// Build PTB for settlement::pay(). Uses tx.coin() to split exact payment amount.
    /// `vault_obj_id` is the recipient peer's PeerVault object ID (where commissions go).
    pub fn build_pay_transaction(
        &self,
        sender: sui_sdk_types::Address,
        recipient: sui_sdk_types::Address,
        blob_hash: &[u8; 32],
        amount: u64,
        vault_obj_id: sui_sdk_types::Address,
    ) -> sui_transaction_builder::TransactionBuilder {
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

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

    /// Build PTB for staking::update_peer_id().
    /// Updates the peer_id on an existing Peer object.
    /// `peer_object_id` and `peer_initial_shared_version` come from fetching
    /// the existing Peer object via RPC.
    pub fn build_update_peer_id_transaction(
        &self,
        sender: sui_sdk_types::Address,
        peer_object_id: sui_sdk_types::Address,
        peer_initial_shared_version: u64,
        new_peer_id: Vec<u8>,
    ) -> sui_transaction_builder::TransactionBuilder {
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

        let registry_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            self.registry_object_id, self.registry_initial_shared_version, false,
        ));
        let peer_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            peer_object_id, peer_initial_shared_version, true,
        ));
        let peer_id_input = tx.pure(&new_peer_id);

        tx.move_call(
            sui_transaction_builder::Function::new(
                self.package_id,
                sui_sdk_types::Identifier::from_static("staking"),
                sui_sdk_types::Identifier::from_static("update_peer_id"),
            ),
            vec![registry_input, peer_input, peer_id_input],
        );

        tx.set_sender(sender);
        tx.set_gas_budget(5_000_000);
        tx
    }

    /// Build PTB for peer_vault::claim_earnings().
    /// Claims the peer's accumulated commission from their vault.
    /// `vault_obj_id` is the peer's PeerVault object ID.
    /// `vault_version` is the current version of the vault object.
    pub fn build_claim_earnings_transaction(
        &self,
        sender: sui_sdk_types::Address,
        vault_obj_id: sui_sdk_types::Address,
        vault_version: u64,
    ) -> sui_transaction_builder::TransactionBuilder {
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

        let vault_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            vault_obj_id, vault_version, true,
        ));

        tx.move_call(
            sui_transaction_builder::Function::new(
                self.package_id,
                sui_sdk_types::Identifier::from_static("peer_vault"),
                sui_sdk_types::Identifier::from_static("claim_earnings"),
            ),
            vec![vault_input],
        );

        tx.set_sender(sender);
        tx.set_gas_budget(5_000_000);
        tx
    }

    /// Build PTB for peer_vault::undelegate().
    /// Burns a ShareCertificate and returns WAL at current exchange rate.
    /// `vault_obj_id` is the PeerVault object ID, `vault_version` its current version.
    /// `cert_obj_id`, `cert_version`, and `cert_digest` identify the owned ShareCertificate.
    pub fn build_undelegate_transaction(
        &self,
        sender: sui_sdk_types::Address,
        vault_obj_id: sui_sdk_types::Address,
        vault_version: u64,
        cert_obj_id: sui_sdk_types::Address,
        cert_version: u64,
        cert_digest: sui_sdk_types::Digest,
    ) -> sui_transaction_builder::TransactionBuilder {
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

        let vault_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            vault_obj_id, vault_version, true,
        ));
        let cert_input = tx.object(sui_transaction_builder::ObjectInput::owned(
            cert_obj_id, cert_version, cert_digest,
        ));

        tx.move_call(
            sui_transaction_builder::Function::new(
                self.package_id,
                sui_sdk_types::Identifier::from_static("peer_vault"),
                sui_sdk_types::Identifier::from_static("undelegate"),
            ),
            vec![vault_input, cert_input],
        );

        tx.set_sender(sender);
        tx.set_gas_budget(5_000_000);
        tx
    }

    /// Build PTB for peer_vault::delegate().
    /// Delegates WAL into a peer's vault and receives a ShareCertificate.
    /// Uses tx.coin() to auto-select and split the exact WAL amount.
    pub fn build_delegate_transaction(
        &self,
        sender: sui_sdk_types::Address,
        vault_obj_id: sui_sdk_types::Address,
        vault_version: u64,
        amount: u64,
    ) -> sui_transaction_builder::TransactionBuilder {
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

        let coin_arg = tx.intent(
            sui_transaction_builder::intent::CoinWithBalance::new(self.wal_struct_tag(), amount)
        );

        let vault_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            vault_obj_id, vault_version, true,
        ));

        tx.move_call(
            sui_transaction_builder::Function::new(
                self.package_id,
                sui_sdk_types::Identifier::from_static("peer_vault"),
                sui_sdk_types::Identifier::from_static("delegate"),
            ),
            vec![vault_input, coin_arg],
        );

        tx.set_sender(sender);
        tx.set_gas_budget(5_000_000);
        tx
    }

    /// Build PTB for peer_vault::new_vault().
    pub fn build_create_vault_transaction(
        &self,
        sender: sui_sdk_types::Address,
    ) -> Option<sui_transaction_builder::TransactionBuilder> {
        let registry_id = self.peer_vault_registry_id?;
        let mut tx = sui_transaction_builder::TransactionBuilder::new();

        let registry_input = tx.object(sui_transaction_builder::ObjectInput::shared(
            registry_id, self.peer_vault_registry_initial_shared_version, true,
        ));

        tx.move_call(
            sui_transaction_builder::Function::new(
                self.package_id,
                sui_sdk_types::Identifier::from_static("peer_vault"),
                sui_sdk_types::Identifier::from_static("new_vault"),
            ),
            vec![registry_input],
        );

        tx.set_sender(sender);
        tx.set_gas_budget(5_000_000);
        Some(tx)
    }
}
