// Copyright (c) Zing CDN
// SPDX-License-Identifier: Apache-2.0

/// Settlement contract. Routes client payments to the serving peer's vault.
/// Maintains an admin-controllable read price (WAL per megabyte) that
/// follows walrus write pricing (target: write_price / 10).
///
/// Because walrus's `write_price_per_unit_size` is not publicly queryable
/// (it's #[test_only]), we expose our own admin-controlled read_price here.
module zing_cdn::settlement;

use wal::wal::WAL;
use sui::coin;
use sui::event;
use zing_cdn::peer_vault;

/// Max read price: 1 WAL per MB (1_000_000_000 frost/MB)
const MAX_READ_PRICE_PER_MB: u64 = 1_000_000_000;

/// Global settlement contract. Admin can update the read price to track
/// WAL/USD rate and maintain target ~$0.0023/GB (= write_price / 10).
public struct Settlement has key {
    id: UID,
    read_price_per_mb: u64,        // WAL frost per megabyte read
}

/// Cap for admin to update the read price.
public struct AdminCap has key {
    id: UID,
}

// ===== Init =====

fun init(ctx: &mut TxContext) {
    transfer::share_object(Settlement {
        id: object::new(ctx),
        read_price_per_mb: 1_000,
    });
    transfer::transfer(AdminCap { id: object::new(ctx) }, ctx.sender());
}

#[test_only]
/// Test helper: initializes settlement without calling private module init.
public fun init_for_testing(ctx: &mut TxContext) {
    transfer::share_object(Settlement {
        id: object::new(ctx),
        read_price_per_mb: 1_000,
    });
    transfer::transfer(AdminCap { id: object::new(ctx) }, ctx.sender());
}

// ===== Payment =====

/// Client pays WAL for a blob fetch from a specific peer.
/// Routes 100% of payment to the serving peer's vault.
/// `payee` is recorded in the event for analytics (no direct transfer).
public fun pay(
    _settlement: &Settlement,
    vault: &mut peer_vault::PeerVault,
    payee: address,
    blob_hash: vector<u8>,
    coin: coin::Coin<WAL>,
    ctx: &mut TxContext,
) {
    let amount = coin::value(&coin);
    assert!(amount > 0, EB_ZERO_PAYMENT);

    // Route payment to peer's vault → commission + yield for delegators
    peer_vault::collect_rewards(vault, coin, ctx);

    event::emit(PaymentEvent {
        payer: ctx.sender(),
        payee,
        blob_hash,
        amount,
    });
}

// ===== Price management =====

/// Compute the fee (in WAL frost) for reading `size_mb` megabytes.
/// The Rust client calls this to determine how much WAL to pay.
public fun compute_fee(settlement: &Settlement, size_mb: u64): u64 {
    assert!(size_mb > 0, EB_ZERO_PAYMENT);
    size_mb * settlement.read_price_per_mb
}

/// Admin updates the read price (in WAL frost per megabyte).
/// Follows walrus write price model: target ~$0.0023/GB (= write / 10).
public fun set_read_price(
    _cap: &AdminCap,
    settlement: &mut Settlement,
    price_per_mb: u64,
) {
    assert!(price_per_mb > 0, 0);
    assert!(price_per_mb <= MAX_READ_PRICE_PER_MB, 1);

    event::emit(ReadPriceUpdatedEvent {
        old_price: settlement.read_price_per_mb,
        new_price: price_per_mb,
    });

    settlement.read_price_per_mb = price_per_mb;
}

/// View: current read price (WAL frost per megabyte).
public fun read_price_per_mb(settlement: &Settlement): u64 {
    settlement.read_price_per_mb
}

// ===== Events =====

public struct PaymentEvent has copy, drop {
    payer: address,
    payee: address,
    blob_hash: vector<u8>,
    amount: u64,
}

public struct ReadPriceUpdatedEvent has copy, drop {
    old_price: u64,
    new_price: u64,
}

// ===== Error codes =====

const EB_ZERO_PAYMENT: u64 = 0;


