#[test_only]
module zing_cdn::integration_tests;

#[test_only]
use sui::test_scenario;
#[test_only]
use sui::coin::{Self, Coin};
#[test_only]
use zing_cdn::staking;
#[test_only]
use zing_cdn::peer_vault;
#[test_only]
use zing_cdn::settlement;

const PEER: address = @0x1;
const DELEGATOR: address = @0x2;
const CLIENT: address = @0x3;
const ADMIN: address = @0x4;

// ===== Test 1: Full economic flow =====

#[test]
fun test_full_flow() {
    let mut s = test_scenario::begin(ADMIN);

    // Init WAL
    test_scenario::next_tx(&mut s, ADMIN);
    {
        wal::wal::init_for_testing(s.ctx());
    };

    // Init staking + settlement
    test_scenario::next_tx(&mut s, ADMIN);
    {
        staking::init_for_testing(s.ctx());
        settlement::init_for_testing(s.ctx());
    };

    // Admin distributes WAL
    test_scenario::next_tx(&mut s, ADMIN);
    {
        let mut admin_wal = test_scenario::take_from_sender<Coin<wal::wal::WAL>>(&s);
        let bond = coin::split(&mut admin_wal, 2_000_000_000_000u64, s.ctx());
        transfer::public_transfer(bond, PEER);
        let del_coin = coin::split(&mut admin_wal, 10_000_000_000_000u64, s.ctx());
        transfer::public_transfer(del_coin, DELEGATOR);
        let pay_coin = coin::split(&mut admin_wal, 2_000_000_000u64, s.ctx());
        transfer::public_transfer(pay_coin, CLIENT);
        test_scenario::return_to_sender(&s, admin_wal);
    };

    // --- Step 1: Peer registers with 2000 WAL bond ---
    test_scenario::next_tx(&mut s, PEER);
    {
        let bond_coin = test_scenario::take_from_sender<Coin<wal::wal::WAL>>(&s);
        let bond_value = coin::value(&bond_coin);
        assert!(bond_value == 2_000_000_000_000u64, 0);

        let mut registry = test_scenario::take_shared<staking::Registry>(&s);
        staking::register_for_testing(&mut registry, b"peer-1234", bond_coin, s.ctx());
        test_scenario::return_shared(registry);
    };

    test_scenario::next_tx(&mut s, ADMIN);
    {
        let peer = test_scenario::take_shared<staking::Peer>(&s);
        assert!(staking::is_active(&peer), 1);
        assert!(staking::total_stake(&peer) == 2_000_000_000_000u64, 2);
        test_scenario::return_shared(peer);
    };

    // --- Step 2: Peer creates vault (10% commission) ---
    test_scenario::next_tx(&mut s, PEER);
    {
        peer_vault::create_vault_for_testing(PEER, 1000u64, s.ctx());
    };

    test_scenario::next_tx(&mut s, ADMIN);
    {
        let vault = test_scenario::take_shared<peer_vault::PeerVault>(&s);
        assert!(peer_vault::total_shares(&vault) == 0, 3);
        assert!(peer_vault::total_reserves(&vault) == 0, 4);
        assert!(peer_vault::exchange_rate(&vault) == 1_000_000_000u64, 5);
        test_scenario::return_shared(vault);
    };

    // --- Step 3: Delegator deposits 10000 WAL ---
    let del_amount;
    test_scenario::next_tx(&mut s, DELEGATOR);
    {
        let del_coin = test_scenario::take_from_sender<Coin<wal::wal::WAL>>(&s);
        del_amount = coin::value(&del_coin);

        let mut vault = test_scenario::take_shared<peer_vault::PeerVault>(&s);
        peer_vault::delegate_for_testing(&mut vault, del_coin, s.ctx());
        assert!(peer_vault::total_shares(&vault) == del_amount, 6);
        assert!(peer_vault::total_reserves(&vault) == del_amount, 7);
        test_scenario::return_shared(vault);
    };

    test_scenario::next_tx(&mut s, DELEGATOR);
    {
        let cert = test_scenario::take_from_sender<peer_vault::ShareCertificate>(&s);
        assert!(peer_vault::cert_shares(&cert) == del_amount, 8);
        test_scenario::return_to_sender(&s, cert);
    };

    // --- Step 4: Client pays 2 WAL via settlement ---
    let pay_amount;
    test_scenario::next_tx(&mut s, CLIENT);
    {
        let pay_coin = test_scenario::take_from_sender<Coin<wal::wal::WAL>>(&s);
        pay_amount = coin::value(&pay_coin);

        let settlement_obj = test_scenario::take_shared<settlement::Settlement>(&s);
        let mut vault = test_scenario::take_shared<peer_vault::PeerVault>(&s);
        settlement::pay_for_testing(&settlement_obj, &mut vault, PEER, b"blob-hash", pay_coin, s.ctx());
        assert!(peer_vault::peer_earnings(&vault) == 200_000_000u64, 9);
        assert!(peer_vault::total_reserves(&vault) == del_amount + pay_amount - 200_000_000u64, 10);
        test_scenario::return_shared(vault);
        test_scenario::return_shared(settlement_obj);
    };

    // --- Step 5: Peer claims 0.2 WAL commission ---
    test_scenario::next_tx(&mut s, PEER);
    {
        let mut vault = test_scenario::take_shared<peer_vault::PeerVault>(&s);
        peer_vault::claim_earnings_for_testing(&mut vault, s.ctx());
        test_scenario::return_shared(vault);
    };

    test_scenario::next_tx(&mut s, PEER);
    {
        let claimed = test_scenario::take_from_sender<Coin<wal::wal::WAL>>(&s);
        assert!(coin::value(&claimed) == 200_000_000u64, 11);
        test_scenario::return_to_sender(&s, claimed);
    };

    // --- Step 6: Delegator undelegates with yield ---
    test_scenario::next_tx(&mut s, DELEGATOR);
    {
        let cert = test_scenario::take_from_sender<peer_vault::ShareCertificate>(&s);
        let mut vault = test_scenario::take_shared<peer_vault::PeerVault>(&s);
        peer_vault::undelegate_for_testing(&mut vault, cert, s.ctx());
        test_scenario::return_shared(vault);
    };

    test_scenario::next_tx(&mut s, DELEGATOR);
    {
        let withdrawn = test_scenario::take_from_sender<Coin<wal::wal::WAL>>(&s);
        assert!(coin::value(&withdrawn) > del_amount, 12);
        assert!(coin::value(&withdrawn) == del_amount + pay_amount - 200_000_000u64, 13);
        test_scenario::return_to_sender(&s, withdrawn);
    };

    test_scenario::end(s);
}

// ===== Test 2: Admin slashing =====

#[test]
fun test_admin_slash() {
    let mut s = test_scenario::begin(ADMIN);

    // Init WAL + staking
    test_scenario::next_tx(&mut s, ADMIN);
    {
        wal::wal::init_for_testing(s.ctx());
    };

    test_scenario::next_tx(&mut s, ADMIN);
    {
        staking::init_for_testing(s.ctx());
    };

    // Admin distributes bond to PEER
    test_scenario::next_tx(&mut s, ADMIN);
    {
        let mut admin_wal = test_scenario::take_from_sender<Coin<wal::wal::WAL>>(&s);
        let bond = coin::split(&mut admin_wal, 5_000_000_000_000u64, s.ctx());
        transfer::public_transfer(bond, PEER);
        test_scenario::return_to_sender(&s, admin_wal);
    };

    // Peer registers
    test_scenario::next_tx(&mut s, PEER);
    {
        let bond_coin = test_scenario::take_from_sender<Coin<wal::wal::WAL>>(&s);
        let mut registry = test_scenario::take_shared<staking::Registry>(&s);
        staking::register_for_testing(&mut registry, b"peer", bond_coin, s.ctx());
        test_scenario::return_shared(registry);
    };

    test_scenario::next_tx(&mut s, ADMIN);
    {
        let peer = test_scenario::take_shared<staking::Peer>(&s);
        assert!(staking::total_stake(&peer) == 5_000_000_000_000u64, 0);
        test_scenario::return_shared(peer);
    };

    // Admin slashes 1000 WAL
    test_scenario::next_tx(&mut s, ADMIN);
    {
        let cap = test_scenario::take_from_sender<staking::AdminCap>(&s);
        let mut peer = test_scenario::take_shared<staking::Peer>(&s);
        staking::slash_for_testing(&cap, &mut peer, 1_000_000_000_000u64, s.ctx());
        test_scenario::return_shared(peer);
        test_scenario::return_to_sender(&s, cap);
    };

    test_scenario::next_tx(&mut s, ADMIN);
    {
        let mut peer = test_scenario::take_shared<staking::Peer>(&s);
        assert!(staking::total_stake(&peer) == 4_000_000_000_000u64, 1);

        // Slashed WAL went to admin
        let slashed = test_scenario::take_from_sender<Coin<wal::wal::WAL>>(&s);
        assert!(coin::value(&slashed) == 1_000_000_000_000u64, 2);

        // Deactivate
        let cap = test_scenario::take_from_sender<staking::AdminCap>(&s);
        staking::deactivate(&cap, &mut peer);
        assert!(!staking::is_active(&peer), 3);

        test_scenario::return_to_sender(&s, cap);
        test_scenario::return_to_sender(&s, slashed);
        test_scenario::return_shared(peer);
    };

    test_scenario::end(s);
}
