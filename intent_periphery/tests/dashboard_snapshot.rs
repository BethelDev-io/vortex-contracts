//! Integration tests for the batched `intent_periphery` view — Issue #534.
//!
//! `intent_periphery` proxies the core settlement contract's views one at a
//! time, so a dashboard that wants several pieces of aggregate state pays one
//! client→periphery round trip *per view*. `get_dashboard_snapshot()` bundles
//! the two views the protocol dashboard always needs together
//! (`get_stats` + `get_protocol_health`, per issue #494) into a single call.
//!
//! These tests drive the periphery against a mock core contract and check:
//!
//! * the snapshot carries both bundled views, under the documented keys,
//! * each bundled value is byte-for-byte the value the corresponding
//!   individual proxy returns,
//! * the existing individual proxies still work (the change is additive),
//! * the snapshot is read-only,
//! * the batched call really is cheaper than two unbatched calls,
//! * and the periphery fails cleanly when it has not been initialized.

use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, xdr::ToXdr, Address, Bytes, Env,
    IntoVal, Map, Symbol, Val,
};
use vortex_intent_periphery::{
    IntentPeriphery, IntentPeripheryClient, SNAPSHOT_KEY_PROTOCOL_HEALTH, SNAPSHOT_KEY_STATS,
};

// ── Mock core settlement contract ─────────────────────────────────────────────
//
// Mirrors the two core views the periphery proxies, with the same shapes the
// real `intent_settlement` contract uses:
//
//   get_stats()           -> (u64, i128, u64)
//   get_protocol_health() -> ProtocolHealth

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolHealth {
    pub paused: bool,
    pub total_intents: u64,
    pub total_volume: i128,
    pub total_solvers: u32,
    pub total_bonded: i128,
}

#[contract]
pub struct MockSettlement;

#[contractimpl]
impl MockSettlement {
    pub fn get_stats(env: Env) -> (u64, i128, u64) {
        (
            env.storage()
                .instance()
                .get(&Symbol::new(&env, "intents"))
                .unwrap_or(0),
            env.storage()
                .instance()
                .get(&Symbol::new(&env, "volume"))
                .unwrap_or(0),
            env.storage()
                .instance()
                .get(&Symbol::new(&env, "open"))
                .unwrap_or(0),
        )
    }

    pub fn get_protocol_health(env: Env) -> ProtocolHealth {
        ProtocolHealth {
            paused: env
                .storage()
                .instance()
                .get(&Symbol::new(&env, "paused"))
                .unwrap_or(false),
            total_intents: env
                .storage()
                .instance()
                .get(&Symbol::new(&env, "intents"))
                .unwrap_or(0),
            total_volume: env
                .storage()
                .instance()
                .get(&Symbol::new(&env, "volume"))
                .unwrap_or(0),
            total_solvers: env
                .storage()
                .instance()
                .get(&Symbol::new(&env, "solvers"))
                .unwrap_or(0),
            total_bonded: env
                .storage()
                .instance()
                .get(&Symbol::new(&env, "bonded"))
                .unwrap_or(0),
        }
    }

    /// Test helper: seed the counters the two views read.
    pub fn set_counters(
        env: Env,
        intents: u64,
        volume: i128,
        open: u64,
        solvers: u32,
        bonded: i128,
        paused: bool,
    ) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "intents"), &intents);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "volume"), &volume);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "open"), &open);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "solvers"), &solvers);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "bonded"), &bonded);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "paused"), &paused);
    }
}

// ── Fixture ────────────────────────────────────────────────────────────────────

struct Ctx {
    env: Env,
    admin: Address,
    settlement: Address,
    periphery: Address,
}

impl Ctx {
    fn client(&self) -> IntentPeripheryClient<'_> {
        IntentPeripheryClient::new(&self.env, &self.periphery)
    }

    fn core(&self) -> MockSettlementClient<'_> {
        MockSettlementClient::new(&self.env, &self.settlement)
    }
}

fn setup() -> Ctx {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let settlement = env.register_contract(None, MockSettlement);
    let periphery = env.register_contract(None, IntentPeriphery);

    let ctx = Ctx {
        env,
        admin,
        settlement,
        periphery,
    };

    ctx.client().initialize(&ctx.admin, &ctx.settlement);
    ctx
}

/// Decode the raw `get_stats()` value into the core's `(u64, i128, u64)`.
fn decode_stats(env: &Env, val: &Val) -> (u64, i128, u64) {
    <(u64, i128, u64) as soroban_sdk::TryFromVal<Env, Val>>::try_from_val(env, val).unwrap()
}

/// Decode the raw `get_protocol_health()` value into the core's struct.
fn decode_health(env: &Env, val: &Val) -> ProtocolHealth {
    <ProtocolHealth as soroban_sdk::TryFromVal<Env, Val>>::try_from_val(env, val).unwrap()
}

/// Canonical byte representation of a raw `Val`.
///
/// `Val` deliberately does not implement `PartialEq`, so the only way to say
/// "these two raw values are the same value" is to compare their
/// serializations. Two `Val`s produced by two separate cross-contract calls
/// are different host objects, so comparing the handles would be wrong; the
/// serialized form is the value itself.
fn val_bytes(env: &Env, val: &Val) -> Bytes {
    val.to_xdr(env)
}

/// Pull one of the bundled views out of the snapshot.
fn snapshot_entry(env: &Env, snapshot: &Map<Symbol, Val>, key: &str) -> Val {
    snapshot
        .get(Symbol::new(env, key))
        .unwrap_or_else(|| panic!("snapshot is missing the {key:?} entry"))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

/// The snapshot carries both bundled views, and each one decodes to exactly
/// what the core returned.
#[test]
fn get_dashboard_snapshot_bundles_stats_and_protocol_health() {
    let ctx = setup();
    ctx.core()
        .set_counters(&42, &1_234_567_890, &7, &3, &999, &false);

    let snapshot = ctx.client().get_dashboard_snapshot();

    let stats = snapshot_entry(&ctx.env, &snapshot, SNAPSHOT_KEY_STATS);
    let (intents, volume, open) = decode_stats(&ctx.env, &stats);
    assert_eq!(intents, 42);
    assert_eq!(volume, 1_234_567_890);
    assert_eq!(open, 7);

    let health = snapshot_entry(&ctx.env, &snapshot, SNAPSHOT_KEY_PROTOCOL_HEALTH);
    let health = decode_health(&ctx.env, &health);
    assert!(!health.paused);
    assert_eq!(health.total_intents, 42);
    assert_eq!(health.total_volume, 1_234_567_890);
    assert_eq!(health.total_solvers, 3);
    assert_eq!(health.total_bonded, 999);
}

/// The bundle holds exactly the two documented views — no more, no less.
///
/// This pins the bundle shape so a future change that adds or renames an
/// entry has to be a deliberate, visible change (and so a client that reads
/// the snapshot by key cannot silently miss a view).
#[test]
fn get_dashboard_snapshot_has_exactly_the_documented_keys() {
    let ctx = setup();

    let snapshot = ctx.client().get_dashboard_snapshot();

    assert_eq!(snapshot.len(), 2);
    let mut keys: Vec<String> = snapshot
        .keys()
        .iter()
        .map(|k| k.to_string())
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            SNAPSHOT_KEY_PROTOCOL_HEALTH.to_string(),
            SNAPSHOT_KEY_STATS.to_string(),
        ]
    );
}

/// The bundled values are the *same* values the individual proxies return.
///
/// This is the test that would catch a plausible wrong implementation: a
/// bundle that re-derived, cached or re-ordered the values would still pass
/// "the snapshot contains stats and health". Here each bundled value is
/// compared, byte for byte, against the value the corresponding individual
/// proxy returns.
#[test]
fn get_dashboard_snapshot_matches_the_individual_proxies() {
    let ctx = setup();
    ctx.core().set_counters(&11, &222, &33, &4, &555, &true);

    let snapshot = ctx.client().get_dashboard_snapshot();

    let stats_from_proxy = ctx.client().get_stats();
    let health_from_proxy = ctx.client().get_protocol_health();

    let bundled_stats = snapshot_entry(&ctx.env, &snapshot, SNAPSHOT_KEY_STATS);
    let bundled_health = snapshot_entry(&ctx.env, &snapshot, SNAPSHOT_KEY_PROTOCOL_HEALTH);

    assert_eq!(
        val_bytes(&ctx.env, &bundled_stats),
        val_bytes(&ctx.env, &stats_from_proxy)
    );
    assert_eq!(
        val_bytes(&ctx.env, &bundled_health),
        val_bytes(&ctx.env, &health_from_proxy)
    );

    // And the individual proxies still decode to the core's own values, i.e.
    // they were not broken by the addition.
    assert_eq!(decode_stats(&ctx.env, &stats_from_proxy), (11, 222, 33));
    assert!(decode_health(&ctx.env, &health_from_proxy).paused);
}

/// The existing individual proxies are still present — the batched view is
/// additive, not a replacement.
#[test]
fn the_individual_proxies_still_work() {
    let ctx = setup();
    ctx.core().set_counters(&5, &6, &7, &8, &9, &false);

    assert_eq!(decode_stats(&ctx.env, &ctx.client().get_stats()), (5, 6, 7));
    assert_eq!(
        decode_health(&ctx.env, &ctx.client().get_protocol_health()).total_solvers,
        8
    );
}

/// The snapshot reflects the core's *current* state, not a value cached at
/// initialization time.
#[test]
fn get_dashboard_snapshot_reflects_live_core_state() {
    let ctx = setup();
    ctx.core().set_counters(&1, &2, &3, &4, &5, &false);

    let before = ctx.client().get_dashboard_snapshot();
    assert_eq!(
        decode_stats(
            &ctx.env,
            &snapshot_entry(&ctx.env, &before, SNAPSHOT_KEY_STATS)
        ),
        (1, 2, 3)
    );

    // The core's counters move on.
    ctx.core().set_counters(&10, &20, &30, &40, &50, &true);

    let after = ctx.client().get_dashboard_snapshot();
    assert_eq!(
        decode_stats(
            &ctx.env,
            &snapshot_entry(&ctx.env, &after, SNAPSHOT_KEY_STATS)
        ),
        (10, 20, 30)
    );
    assert!(
        decode_health(
            &ctx.env,
            &snapshot_entry(&ctx.env, &after, SNAPSHOT_KEY_PROTOCOL_HEALTH)
        )
        .paused
    );

    // The snapshot really did change — the two bundles are not the same value.
    assert_ne!(
        val_bytes(
            &ctx.env,
            &snapshot_entry(&ctx.env, &before, SNAPSHOT_KEY_STATS)
        ),
        val_bytes(
            &ctx.env,
            &snapshot_entry(&ctx.env, &after, SNAPSHOT_KEY_STATS)
        )
    );
}

/// The snapshot view is read-only: calling it repeatedly returns the same
/// value and does not mutate the core's state.
#[test]
fn get_dashboard_snapshot_is_read_only() {
    let ctx = setup();
    ctx.core().set_counters(&3, &4, &5, &6, &7, &false);

    let first = ctx.client().get_dashboard_snapshot();
    for _ in 0..5 {
        assert_eq!(ctx.client().get_dashboard_snapshot(), first);
    }

    // The core's counters are untouched by the reads.
    assert_eq!(
        decode_stats(
            &ctx.env,
            &snapshot_entry(&ctx.env, &first, SNAPSHOT_KEY_STATS)
        ),
        (3, 4, 5)
    );
    assert_eq!(
        decode_health(
            &ctx.env,
            &snapshot_entry(&ctx.env, &first, SNAPSHOT_KEY_PROTOCOL_HEALTH)
        )
        .total_solvers,
        6
    );
}

/// The batched view really does collapse two client→periphery round trips
/// into one.
///
/// This is the property the issue is actually about ("N separate
/// cross-contract round trips through the periphery, one per view" → "a
/// single cross-contract round trip"), so it is asserted directly: the host
/// budget consumed by one `get_dashboard_snapshot()` call is strictly lower
/// than the budget consumed by calling `get_stats()` and
/// `get_protocol_health()` separately.
#[test]
fn get_dashboard_snapshot_costs_less_than_two_individual_proxies() {
    let ctx = setup();
    ctx.core().set_counters(&1, &2, &3, &4, &5, &false);

    let mut budget = ctx.env.budget();

    budget.reset_tracker();
    ctx.client().get_dashboard_snapshot();
    let batched_cost = budget.cpu_instruction_cost();

    budget.reset_tracker();
    ctx.client().get_stats();
    ctx.client().get_protocol_health();
    let unbatched_cost = budget.cpu_instruction_cost();

    assert!(
        batched_cost < unbatched_cost,
        "batched view must cost less than two unbatched calls \
         (batched: {batched_cost} cpu insns, unbatched: {unbatched_cost} cpu insns)"
    );
}

/// The periphery reports the same `NotInitialized` error for the batched view
/// as it does for the individual proxies, when it has not been initialized.
#[test]
fn get_dashboard_snapshot_fails_before_initialization() {
    let env = Env::default();
    env.mock_all_auths();
    let periphery = env.register_contract(None, IntentPeriphery);
    let client = IntentPeripheryClient::new(&env, &periphery);

    assert_eq!(
        client.try_get_dashboard_snapshot(),
        Err(Ok(soroban_sdk::Error::from(
            vortex_intent_periphery::Error::NotInitialized
        )))
    );
}

/// The batched view is decodable by a client using the core's published ABI:
/// each bundled entry is the core's own value, untouched by the periphery.
#[test]
fn get_dashboard_snapshot_entries_decode_with_the_core_abi() {
    let ctx = setup();
    ctx.core().set_counters(&2, &20, &200, &2, &20, &false);

    let snapshot = ctx.client().get_dashboard_snapshot();

    let stats = snapshot_entry(&ctx.env, &snapshot, SNAPSHOT_KEY_STATS);
    let health = snapshot_entry(&ctx.env, &snapshot, SNAPSHOT_KEY_PROTOCOL_HEALTH);

    assert_eq!(decode_stats(&ctx.env, &stats), (2, 20, 200));
    assert_eq!(
        decode_health(&ctx.env, &health),
        ProtocolHealth {
            paused: false,
            total_intents: 2,
            total_volume: 20,
            total_solvers: 2,
            total_bonded: 20,
        }
    );

    // The raw entries still round-trip through the ABI as opaque values, so
    // a client that does not know the core's struct layout can forward them
    // untouched.
    let stats_again = <Val as soroban_sdk::TryFromVal<Env, Val>>::try_from_val(
        &ctx.env,
        &stats.into_val(&ctx.env),
    )
    .unwrap();
    assert_eq!(
        val_bytes(&ctx.env, &stats_again),
        val_bytes(&ctx.env, &stats)
    );
}
