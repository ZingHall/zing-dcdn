// Copyright (c) Zing CDN
// SPDX-License-Identifier: Apache-2.0

/// Core PoS registry. Tracks peer registration, self-stake bonds, and
/// provides slashing via admin cap. No rewards logic — that belongs in
/// the periphery (`peer_vault`).
module zing_cdn::staking;

use wal::wal::WAL;
use sui::balance::{Self, Balance};
use sui::coin::{Self, Coin};
use sui::event;
use sui::table::{Self, Table};

/// Minimum self-stake bond to register as a peer (in frost, 9 decimals).
/// 1000 WAL = 1_000_000_000_000 frost.
const MIN_STAKE: u64 = 1_000_000_000_000;

// ===== Shared objects =====

/// Global registry of all registered peers.
public struct Registry has key, store {
    id: UID,
    peers: Table<address, ID>,   // sui_address → Peer object ID
}

/// A registered peer's staking position (the security bond).
public struct Peer has key, store {
    id: UID,
    peer_id: vector<u8>,         // libp2p PeerId bytes
    sui_address: address,        // peer's Sui wallet (registered by)
    bond: Balance<WAL>,          // self-stake (reclaimable, slashable)
    is_active: bool,
}

/// Capability for admin operations (slash, deactivate).
public struct AdminCap has key, store {
    id: UID,
}

// ===== Init =====

fun init(ctx: &mut TxContext) {
    transfer::public_share_object(Registry {
        id: object::new(ctx),
        peers: table::new(ctx),
    });
    transfer::transfer(AdminCap { id: object::new(ctx) }, ctx.sender());
}

#[test_only]
/// Test helper: initializes staking without calling private module init.
public fun init_for_testing(ctx: &mut TxContext) {
    transfer::public_share_object(Registry {
        id: object::new(ctx),
        peers: table::new(ctx),
    });
    transfer::transfer(AdminCap { id: object::new(ctx) }, ctx.sender());
}

// ===== Peer registration =====

/// Register as a peer with minimum required bond.
/// The `peer_id` is the libp2p PeerId in raw bytes (length-prefixed multihash).
public fun register(
    registry: &mut Registry,
    peer_id: vector<u8>,
    bond: Coin<WAL>,
    ctx: &mut TxContext,
) {
    let sender = ctx.sender();
    assert!(!table::contains(&registry.peers, sender), EB_ALREADY_REGISTERED);
    assert!(coin::value(&bond) >= MIN_STAKE, EB_INSUFFICIENT_BOND);

    let peer = Peer {
        id: object::new(ctx),
        peer_id,
        sui_address: sender,
        bond: coin::into_balance(bond),
        is_active: true,
    };

    let peer_id_obj = object::id(&peer);
    table::add(&mut registry.peers, sender, peer_id_obj);
    transfer::public_share_object(peer);

    event::emit(RegisterEvent {
        peer: sender,
        peer_id,
    });
}

// ===== Stake management =====

/// Add more self-stake to an existing peer bond.
public fun add_stake(
    _registry: &Registry,
    peer: &mut Peer,
    coin: Coin<WAL>,
) {
    balance::join(&mut peer.bond, coin::into_balance(coin));
    event::emit(StakeEvent {
        peer: peer.sui_address,
        amount: balance::value(&peer.bond),
    });
}

/// Unstake part of the bond. Must leave at least MIN_STAKE.
public fun unstake(
    _registry: &Registry,
    peer: &mut Peer,
    amount: u64,
    ctx: &mut TxContext,
) {
    assert!(ctx.sender() == peer.sui_address, EB_NOT_PEER_OWNER);

    let current = balance::value(&peer.bond);
    assert!(current >= amount, EB_INSUFFICIENT_STAKE);
    assert!(current - amount >= MIN_STAKE, EB_BELOW_MIN_STAKE);

    let withdrawn = balance::split(&mut peer.bond, amount);
    transfer::public_transfer(coin::from_balance(withdrawn, ctx), ctx.sender());

    event::emit(UnstakeEvent {
        peer: peer.sui_address,
        amount,
    });
}

// ===== Admin operations =====

/// Slash a portion of a peer's bond. Only called by admin cap holder.
/// Slashed WAL is transferred to the admin as enforcement reward.
public fun slash(
    _cap: &AdminCap,
    peer: &mut Peer,
    amount: u64,
    ctx: &mut TxContext,
) {
    assert!(balance::value(&peer.bond) >= amount, EB_INSUFFICIENT_STAKE);
    let slashed = balance::split(&mut peer.bond, amount);
    transfer::public_transfer(coin::from_balance(slashed, ctx), ctx.sender());

    event::emit(SlashEvent {
        peer: peer.sui_address,
        amount,
    });
}

/// Deactivate a peer (prevents new routes). Admin only.
public fun deactivate(
    _cap: &AdminCap,
    peer: &mut Peer,
) {
    peer.is_active = false;
}

// ===== View functions =====

/// Total self-stake of a peer (excluding delegation).
/// Used by clients to determine routing priority.
public fun total_stake(peer: &Peer): u64 {
    balance::value(&peer.bond)
}

public fun is_active(peer: &Peer): bool {
    peer.is_active
}

public fun peer_id(peer: &Peer): &vector<u8> {
    &peer.peer_id
}

public fun sui_address(peer: &Peer): address {
    peer.sui_address
}

/// Look up a peer's object ID by Sui address.
public fun find_peer(registry: &Registry, addr: address): Option<ID> {
    if (table::contains(&registry.peers, addr)) {
        option::some(*table::borrow(&registry.peers, addr))
    } else {
        option::none()
    }
}

// ===== Events =====

public struct RegisterEvent has copy, drop {
    peer: address,
    peer_id: vector<u8>,
}

public struct StakeEvent has copy, drop {
    peer: address,
    amount: u64,
}

public struct UnstakeEvent has copy, drop {
    peer: address,
    amount: u64,
}

public struct SlashEvent has copy, drop {
    peer: address,
    amount: u64,
}

// ===== Error codes =====

const EB_ALREADY_REGISTERED: u64 = 0;
const EB_INSUFFICIENT_BOND: u64 = 1;
const EB_INSUFFICIENT_STAKE: u64 = 2;
const EB_BELOW_MIN_STAKE: u64 = 3;
const EB_NOT_PEER_OWNER: u64 = 4;
