// Copyright (c) Zing CDN
// SPDX-License-Identifier: Apache-2.0

/// Periphery LST vaults. Each peer creates a PeerVault. Delegators deposit
/// WAL into a specific peer's vault and receive ShareCertificate receipts
/// that track their proportional ownership of the vault's reserves.
///
/// Rewards from settlement::pay() are routed to the serving peer's vault.
/// Commission goes to the peer; the remainder increases the exchange rate
/// for all ShareCertificate holders.
///
/// MVP: ShareCertificate is an NFT-like receipt (not a Coin).
/// Upgrade path: peers can deploy their own Move package with an OTW to
/// create a true liquid Coin<PeerLST> that wraps their ShareCertificate.
module zing_cdn::peer_vault {
    use sui::{balance::{Self, Balance}, coin::{Self, Coin}, event, table::{Self, Table}};
    use wal::wal::WAL;
    use zing_cdn::{staking::AdminCap, utils};

    // ===== Shared object =====

    public struct PeerVaultRegistry has key, store {
        id: UID,
        peer_vaults: Table<address, ID>, // sui_address → PeerVault object ID
    }

    /// A per-peer vault. Created once by a peer to accept delegated WAL
    /// and accumulate settlement rewards.
    public struct PeerVault has key, store {
        id: UID,
        peer_address: address, // peer who owns this vault
        reserves: Balance<WAL>, // total WAL (delegations + rewards)
        total_shares: u64, // total shares outstanding
        commission_bps: u64, // peer's cut in basis points (1000 = 10%)
        peer_earnings: Balance<WAL>, // accumulated (unclaimed) commission
    }

    /// An LST receipt representing a share of a specific PeerVault.
    /// NOT a fungible Coin in MVP. Can be traded or transferred as an NFT.
    /// Upgradeable to per-peer Coin<PeerLST> in the future.
    public struct ShareCertificate has key, store {
        id: UID,
        vault_id: ID, // which vault this belongs to
        shares: u64, // number of shares
    }

    // ===== Vault lifecycle =====

    /// Peer creates their vault. Commission in basis points (max 10000 = 100%).
    public fun create_vault(peer_address: address, commission_bps: u64, ctx: &mut TxContext) {
        assert!(commission_bps <= 10000, EB_INVALID_COMMISSION);

        transfer::public_share_object(PeerVault {
            id: object::new(ctx),
            peer_address,
            reserves: balance::zero<WAL>(),
            total_shares: 0,
            commission_bps,
            peer_earnings: balance::zero<WAL>(),
        });
    }

    public fun new_registry(_cap: &AdminCap, ctx: &mut TxContext) {
        transfer::public_share_object(PeerVaultRegistry {
            id: object::new(ctx),
            peer_vaults: table::new(ctx),
        });
    }

    public fun new_vault(reg: &mut PeerVaultRegistry, ctx: &mut TxContext) {
        let peer_vault = PeerVault {
            id: object::new(ctx),
            peer_address: ctx.sender(),
            reserves: balance::zero<WAL>(),
            total_shares: 0,
            commission_bps: 1000,
            peer_earnings: balance::zero<WAL>(),
        };
        reg.peer_vaults.add(peer_vault.peer_address, object::id(&peer_vault));
        transfer::public_share_object(peer_vault);
    }

    public fun add_vault(
        reg: &mut PeerVaultRegistry,
        _cap: &AdminCap,
        peer_address: address,
        peer_vault_id: ID,
    ) {
        if (reg.peer_vaults.contains(peer_address))
            *&mut reg.peer_vaults[peer_address] = peer_vault_id else reg
            .peer_vaults
            .add(peer_address, peer_vault_id);
    }

    // ===== Delegation =====

    /// Delegator deposits WAL into a peer's vault and receives a
    /// ShareCertificate. The number of shares minted preserves the
    /// current exchange rate.
    public fun delegate(vault: &mut PeerVault, coin: Coin<WAL>, ctx: &mut TxContext) {
        let wal_amount = coin::value(&coin);
        assert!(wal_amount > 0, EB_ZERO_AMOUNT);

        let shares = if (vault.total_shares == 0) {
            wal_amount // 1:1 initial ratio
        } else {
            utils::mul_div(
                wal_amount,
                vault.total_shares,
                balance::value(&vault.reserves),
            )
        };
        assert!(shares > 0, EB_ZERO_SHARES);

        balance::join(&mut vault.reserves, coin::into_balance(coin));
        vault.total_shares = vault.total_shares + shares;

        event::emit(DelegateEvent {
            delegator: ctx.sender(),
            peer: vault.peer_address,
            wal_amount,
            shares,
        });

        transfer::public_transfer(
            ShareCertificate {
                id: object::new(ctx),
                vault_id: object::id(vault),
                shares,
            },
            ctx.sender(),
        );
    }

    /// Delegator burns their ShareCertificate and receives WAL at the
    /// current exchange rate (original deposit + accrued yield).
    public fun undelegate(vault: &mut PeerVault, cert: ShareCertificate, ctx: &mut TxContext) {
        assert!(object::id(vault) == cert.vault_id, EB_WRONG_VAULT);

        let shares = cert.shares;
        assert!(shares > 0, EB_ZERO_AMOUNT);

        let wal_amount = utils::mul_div(
            shares,
            balance::value(&vault.reserves),
            vault.total_shares,
        );

        vault.total_shares = vault.total_shares - shares;
        let withdrawn = balance::split(&mut vault.reserves, wal_amount);

        let ShareCertificate { id, vault_id: _, shares: _ } = cert;
        object::delete(id);

        event::emit(UndelegateEvent {
            delegator: ctx.sender(),
            peer: vault.peer_address,
            shares,
            wal_amount,
        });

        transfer::public_transfer(
            coin::from_balance(withdrawn, ctx),
            ctx.sender(),
        );
    }

    // ===== Rewards =====

    /// Called by settlement when a peer serves data.
    /// Splits payment: commission → peer_earnings, remainder → reserves
    /// (increases exchange rate for all ShareCertificate holders).
    public(package) fun collect_rewards(
        vault: &mut PeerVault,
        coin: Coin<WAL>,
        ctx: &mut TxContext,
    ) {
        let amount = coin::value(&coin);
        assert!(amount > 0, EB_ZERO_AMOUNT);

        let commission = utils::mul_div(amount, vault.commission_bps, 10000);
        let mut payment_coin = coin;
        let peer_cut_coin = coin::split(&mut payment_coin, commission, ctx);

        balance::join(&mut vault.peer_earnings, coin::into_balance(peer_cut_coin));
        balance::join(&mut vault.reserves, coin::into_balance(payment_coin));
        // total_shares unchanged → exchange rate increases for delegators

        event::emit(RewardsEvent {
            peer: vault.peer_address,
            amount,
            commission,
        });
    }

    /// Peer claims their accumulated commission earnings.
    public fun claim_earnings(vault: &mut PeerVault, ctx: &mut TxContext) {
        assert!(ctx.sender() == vault.peer_address, EB_NOT_VAULT_OWNER);

        let amount = balance::value(&vault.peer_earnings);
        assert!(amount > 0, EB_NO_EARNINGS);

        let withdrawn = balance::split(&mut vault.peer_earnings, amount);
        transfer::public_transfer(
            coin::from_balance(withdrawn, ctx),
            ctx.sender(),
        );

        event::emit(EarningsClaimedEvent {
            peer: vault.peer_address,
            amount,
        });
    }

    // ===== View functions =====

    /// Current exchange rate: 1 share = how much WAL (in frost, scaled by 1e9).
    public fun exchange_rate(vault: &PeerVault): u64 {
        if (vault.total_shares == 0) {
            return 1_000_000_000 // 1:1 (scaled)
        };
        utils::mul_div(
            balance::value(&vault.reserves),
            1_000_000_000,
            vault.total_shares,
        )
    }

    public fun total_reserves(vault: &PeerVault): u64 {
        balance::value(&vault.reserves)
    }

    public fun total_shares(vault: &PeerVault): u64 {
        vault.total_shares
    }

    public fun peer_earnings(vault: &PeerVault): u64 {
        balance::value(&vault.peer_earnings)
    }

    public fun commission_bps(vault: &PeerVault): u64 {
        vault.commission_bps
    }

    // ===== ShareCertificate accessors =====

    public fun cert_shares(cert: &ShareCertificate): u64 {
        cert.shares
    }

    public fun cert_vault_id(cert: &ShareCertificate): ID {
        cert.vault_id
    }

    // ===== Events =====

    public struct DelegateEvent has copy, drop {
        delegator: address,
        peer: address,
        wal_amount: u64,
        shares: u64,
    }

    public struct UndelegateEvent has copy, drop {
        delegator: address,
        peer: address,
        shares: u64,
        wal_amount: u64,
    }

    public struct RewardsEvent has copy, drop {
        peer: address,
        amount: u64,
        commission: u64,
    }

    public struct EarningsClaimedEvent has copy, drop {
        peer: address,
        amount: u64,
    }

    // ===== Test helpers =====

    #[test_only]
    public fun create_vault_for_testing(
        peer_address: address,
        commission_bps: u64,
        ctx: &mut TxContext,
    ) {
        create_vault(peer_address, commission_bps, ctx);
    }

    #[test_only]
    public fun delegate_for_testing(vault: &mut PeerVault, coin: Coin<WAL>, ctx: &mut TxContext) {
        let wal_amount = coin::value(&coin);
        assert!(wal_amount > 0, EB_ZERO_AMOUNT);

        let shares = if (vault.total_shares == 0) {
            wal_amount
        } else {
            utils::mul_div(wal_amount, vault.total_shares, balance::value(&vault.reserves))
        };
        assert!(shares > 0, EB_ZERO_SHARES);

        balance::join(&mut vault.reserves, coin::into_balance(coin));
        vault.total_shares = vault.total_shares + shares;

        event::emit(DelegateEvent {
            delegator: ctx.sender(),
            peer: vault.peer_address,
            wal_amount,
            shares,
        });

        transfer::public_transfer(
            ShareCertificate {
                id: object::new(ctx),
                vault_id: object::id(vault),
                shares,
            },
            ctx.sender(),
        );
    }

    #[test_only]
    public fun undelegate_for_testing(
        vault: &mut PeerVault,
        cert: ShareCertificate,
        ctx: &mut TxContext,
    ) {
        assert!(object::id(vault) == cert.vault_id, EB_WRONG_VAULT);

        let shares = cert.shares;
        assert!(shares > 0, EB_ZERO_AMOUNT);

        let wal_amount = utils::mul_div(
            shares,
            balance::value(&vault.reserves),
            vault.total_shares,
        );

        vault.total_shares = vault.total_shares - shares;
        let withdrawn = balance::split(&mut vault.reserves, wal_amount);

        let ShareCertificate { id, vault_id: _, shares: _ } = cert;
        object::delete(id);

        event::emit(UndelegateEvent {
            delegator: ctx.sender(),
            peer: vault.peer_address,
            shares,
            wal_amount,
        });

        transfer::public_transfer(
            coin::from_balance(withdrawn, ctx),
            ctx.sender(),
        );
    }

    #[test_only]
    public fun claim_earnings_for_testing(vault: &mut PeerVault, ctx: &mut TxContext) {
        assert!(ctx.sender() == vault.peer_address, EB_NOT_VAULT_OWNER);

        let amount = balance::value(&vault.peer_earnings);
        assert!(amount > 0, EB_NO_EARNINGS);

        let withdrawn = balance::split(&mut vault.peer_earnings, amount);

        event::emit(EarningsClaimedEvent {
            peer: vault.peer_address,
            amount,
        });

        transfer::public_transfer(
            coin::from_balance(withdrawn, ctx),
            ctx.sender(),
        );
    }

    // ===== Error codes =====

    const EB_ZERO_AMOUNT: u64 = 101;
    const EB_ZERO_SHARES: u64 = 102;
    const EB_WRONG_VAULT: u64 = 103;
    const EB_INVALID_COMMISSION: u64 = 104;
    const EB_NOT_VAULT_OWNER: u64 = 105;
    const EB_NO_EARNINGS: u64 = 106;
}
