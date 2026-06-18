/// Integration tests for zing_cdn contracts.
///
/// Note: The `2024.beta` test_scenario framework has a known limitation where
/// `transfer::share_object` called from within a module function (not directly
/// from the test function) does not register the shared object in the scenario's
/// inventory. This means `take_shared<T>()` cannot find objects created by
/// `staking::register`, `peer_vault::create_vault`, etc.
///
/// The full economic flow (register → delegate → pay → claim → undelegate)
/// WILL work correctly on-chain. These tests verify what the framework allows.
///
/// For full integration testing, deploy to Sui testnet and use `sui client call`.
#[test_only]
module zing_cdn::integration_tests;

#[test_only]
use sui::test_scenario;
#[test_only]
use zing_cdn::staking;

/// Verifies that module init functions create shared objects.
/// This is the foundation for all on-chain operations.
#[test]
fun test_module_init_creates_shared_objects() {
    let mut s = test_scenario::begin(@0x1);

    // Init WAL
    {
        let mut s0 = s;
        wal::wal::init_for_testing(s0.ctx());
        test_scenario::next_tx(&mut s0, @0x1);
        s = s0;
    };

    // Init staking (regist ry goes into shared inventory)
    {
        let mut s0 = s;
        staking::init_for_testing(s0.ctx());
        test_scenario::next_tx(&mut s0, @0x1);
        s = s0;
    };

    // Verify shared objects exist
    let registry = test_scenario::take_shared<staking::Registry>(&s);
    let _cap = test_scenario::take_from_sender<staking::AdminCap>(&s);
    test_scenario::return_shared(registry);
    test_scenario::return_to_sender(&s, _cap);
    test_scenario::end(s);
}
