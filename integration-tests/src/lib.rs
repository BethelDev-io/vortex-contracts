//! Cross-contract integration tests over the real release wasm artifacts.
//!
//! Each contract is imported via `contractimport!` against its release wasm so
//! that auth of `current_contract_address`, ABI drift, and nested-call resource
//! limits are exercised against the actual bytecode rather than mocks.
//!
//! The wasm artifacts must be built before these tests run; see the
//! `integration-test` Makefile target which builds the wasms first.

#![cfg(test)]

use soroban_sdk::{Address, BytesN, Env, Symbol};

// Release wasm artifacts, built by the Makefile target before tests run.
mod registry_wasm {
    soroban_sdk::contractimport!(
        file = "../target/wasm32-unknown-unknown/release/registry.wasm"
    );
}

mod settlement_wasm {
    soroban_sdk::contractimport!(
        file = "../target/wasm32-unknown-unknown/release/intent_settlement.wasm"
    );
}

mod proofs_wasm {
    soroban_sdk::contractimport!(
        file = "../target/wasm32-unknown-unknown/release/proofs.wasm"
    );
}

mod badges_wasm {
    soroban_sdk::contractimport!(
        file = "../target/wasm32-unknown-unknown/release/badges.wasm"
    );
}

/// Typed clients for every contract in the protocol, wired together.
pub struct Protocol {
    pub env: Env,
    pub registry: registry_wasm::Client<'static>,
    pub settlement: settlement_wasm::Client<'static>,
    pub proofs: proofs_wasm::Client<'static>,
    pub badges: badges_wasm::Client<'static>,
    pub admin: Address,
}

/// Deploy all contracts and wire cross-contract references.
pub fn deploy_protocol() -> Protocol {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    let registry_id = env.register_contract_wasm(None, registry_wasm::WASM);
    let settlement_id = env.register_contract_wasm(None, settlement_wasm::WASM);
    let proofs_id = env.register_contract_wasm(None, proofs_wasm::WASM);
    let badges_id = env.register_contract_wasm(None, badges_wasm::WASM);

    let registry = registry_wasm::Client::new(&env, &registry_id);
    let settlement = settlement_wasm::Client::new(&env, &settlement_id);
    let proofs = proofs_wasm::Client::new(&env, &proofs_id);
    let badges = badges_wasm::Client::new(&env, &badges_id);

    registry.initialize(&admin);
    settlement.initialize(&admin, &registry_id, &proofs_id);
    proofs.initialize(&admin, &registry_id);
    badges.initialize(&admin, &registry_id);

    Protocol {
        env,
        registry,
        settlement,
        proofs,
        badges,
        admin,
    }
}

/// Assert that a nested cross-contract call stays within a CPU budget.
fn assert_cpu_budget(env: &Env, before: u64, budget: u64) {
    let used = env.cpu_instructions() - before;
    assert!(
        used <= budget,
        "nested call exceeded CPU budget: used {used}, budget {budget}"
    );
}

fn assert_mem_budget(env: &Env, before: u64, budget: u64) {
    let used = env.memory_usage() - before;
    assert!(
        used <= budget,
        "nested call exceeded memory budget: used {used}, budget {budget}"
    );
}

fn sample_hash(env: &Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
}

// ---------------------------------------------------------------------------
// Scenario 1: happy path across all contracts.
// ---------------------------------------------------------------------------
#[test]
fn scenario_happy_path() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);

    p.registry.register(&user, &1u32);
    let tier = p.registry.tier_of(&user);
    assert_eq!(tier, 1u32);

    let hash = sample_hash(&p.env, 1);
    p.proofs.submit(&user, &hash);
    assert!(p.proofs.is_verified(&user, &hash));

    let fill = p.settlement.fill(&user, &hash, &100i128);
    assert_eq!(fill, 100i128);

    p.badges.sync(&user);
    assert!(p.badges.has_badge(&user, &Symbol::new(&p.env, "tier1")));
}

// ---------------------------------------------------------------------------
// Scenario 2: registry tier affects settlement amount.
// ---------------------------------------------------------------------------
#[test]
fn scenario_tier_affects_settlement() {
    let p = deploy_protocol();
    let low = Address::generate(&p.env);
    let high = Address::generate(&p.env);

    p.registry.register(&low, &1u32);
    p.registry.register(&high, &3u32);

    let hash = sample_hash(&p.env, 2);
    p.proofs.submit(&low, &hash);
    p.proofs.submit(&high, &hash);

    let low_fill = p.settlement.fill(&low, &hash, &100i128);
    let high_fill = p.settlement.fill(&high, &hash, &100i128);
    assert!(high_fill > low_fill, "higher tier must settle more");
}

// ---------------------------------------------------------------------------
// Scenario 3: slash with a tier change.
// ---------------------------------------------------------------------------
#[test]
fn scenario_slash_with_tier_change() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);

    p.registry.register(&user, &3u32);
    assert_eq!(p.registry.tier_of(&user), 3u32);

    p.registry.slash(&user, &1u32);
    assert_eq!(p.registry.tier_of(&user), 2u32);
}

// ---------------------------------------------------------------------------
// Scenario 4: proof mismatch blocks a fill.
// ---------------------------------------------------------------------------
#[test]
fn scenario_proof_mismatch_blocks_fill() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &1u32);

    let good = sample_hash(&p.env, 4);
    let bad = sample_hash(&p.env, 5);
    p.proofs.submit(&user, &good);

    let res = p.settlement.try_fill(&user, &bad, &100i128);
    assert!(res.is_err(), "fill with mismatched proof must fail");
}

// ---------------------------------------------------------------------------
// Scenario 5: dispute upheld triggers a registry slash.
// ---------------------------------------------------------------------------
#[test]
fn scenario_dispute_upheld_slashes_registry() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &3u32);

    let hash = sample_hash(&p.env, 6);
    p.proofs.submit(&user, &hash);
    p.settlement.fill(&user, &hash, &100i128);

    p.settlement.dispute(&user, &hash);
    p.settlement.resolve_dispute(&user, &hash, &true);

    assert_eq!(p.registry.tier_of(&user), 2u32, "upheld dispute must slash");
}

// ---------------------------------------------------------------------------
// Scenario 6: badge sync after demotion.
// ---------------------------------------------------------------------------
#[test]
fn scenario_badge_sync_after_demotion() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &3u32);
    p.badges.sync(&user);
    assert!(p.badges.has_badge(&user, &Symbol::new(&p.env, "tier3")));

    p.registry.slash(&user, &1u32);
    p.badges.sync(&user);
    assert!(!p.badges.has_badge(&user, &Symbol::new(&p.env, "tier3")));
    assert!(p.badges.has_badge(&user, &Symbol::new(&p.env, "tier2")));
}

// ---------------------------------------------------------------------------
// Scenario 7: pause of one contract during a flow.
// ---------------------------------------------------------------------------
#[test]
fn scenario_pause_during_flow() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &1u32);

    let hash = sample_hash(&p.env, 7);
    p.proofs.submit(&user, &hash);

    p.settlement.pause();
    let res = p.settlement.try_fill(&user, &hash, &100i128);
    assert!(res.is_err(), "paused settlement must reject fills");

    p.settlement.unpause();
    let fill = p.settlement.fill(&user, &hash, &100i128);
    assert_eq!(fill, 100i128);
}

// ---------------------------------------------------------------------------
// Scenario 8: upgrade mid-flow preserves state across all contracts.
// ---------------------------------------------------------------------------
#[test]
fn scenario_upgrade_mid_flow_preserves_state() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &2u32);

    let hash = sample_hash(&p.env, 8);
    p.proofs.submit(&user, &hash);

    // Upgrade registry mid-flow to the same wasm (state must survive).
    let new_wasm = registry_wasm::WASM;
    p.registry.upgrade(&new_wasm);

    assert_eq!(p.registry.tier_of(&user), 2u32, "tier must survive upgrade");
    assert!(p.proofs.is_verified(&user, &hash));

    let fill = p.settlement.fill(&user, &hash, &100i128);
    assert_eq!(fill, 100i128);
}

// ---------------------------------------------------------------------------
// Scenario 9: unregistered user cannot fill.
// ---------------------------------------------------------------------------
#[test]
fn scenario_unregistered_cannot_fill() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    let hash = sample_hash(&p.env, 9);
    p.proofs.submit(&user, &hash);

    let res = p.settlement.try_fill(&user, &hash, &100i128);
    assert!(res.is_err(), "unregistered user must not fill");
}

// ---------------------------------------------------------------------------
// Scenario 10: unverified proof cannot fill.
// ---------------------------------------------------------------------------
#[test]
fn scenario_unverified_proof_cannot_fill() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &1u32);

    let hash = sample_hash(&p.env, 10);
    let res = p.settlement.try_fill(&user, &hash, &100i128);
    assert!(res.is_err(), "unverified proof must not fill");
}

// ---------------------------------------------------------------------------
// Scenario 11: dispute rejected leaves tier intact.
// ---------------------------------------------------------------------------
#[test]
fn scenario_dispute_rejected_keeps_tier() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &3u32);

    let hash = sample_hash(&p.env, 11);
    p.proofs.submit(&user, &hash);
    p.settlement.fill(&user, &hash, &100i128);
    p.settlement.dispute(&user, &hash);
    p.settlement.resolve_dispute(&user, &hash, &false);

    assert_eq!(p.registry.tier_of(&user), 3u32);
}

// ---------------------------------------------------------------------------
// Scenario 12: repeated slashes floor at tier 0.
// ---------------------------------------------------------------------------
#[test]
fn scenario_repeated_slashes_floor_at_zero() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &2u32);

    p.registry.slash(&user, &1u32);
    p.registry.slash(&user, &1u32);
    p.registry.slash(&user, &1u32);
    assert_eq!(p.registry.tier_of(&user), 0u32);
}

// ---------------------------------------------------------------------------
// Scenario 13: badge sync is idempotent.
// ---------------------------------------------------------------------------
#[test]
fn scenario_badge_sync_idempotent() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &1u32);

    p.badges.sync(&user);
    p.badges.sync(&user);
    assert!(p.badges.has_badge(&user, &Symbol::new(&p.env, "tier1")));
}

// ---------------------------------------------------------------------------
// Scenario 14: proofs contract paused blocks submission.
// ---------------------------------------------------------------------------
#[test]
fn scenario_proofs_pause_blocks_submission() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &1u32);

    p.proofs.pause();
    let hash = sample_hash(&p.env, 14);
    let res = p.proofs.try_submit(&user, &hash);
    assert!(res.is_err(), "paused proofs must reject submissions");

    p.proofs.unpause();
    p.proofs.submit(&user, &hash);
    assert!(p.proofs.is_verified(&user, &hash));
}

// ---------------------------------------------------------------------------
// Scenario 15: resource budget on nested fill call.
// ---------------------------------------------------------------------------
#[test]
fn scenario_nested_call_resource_budget() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &2u32);

    let hash = sample_hash(&p.env, 15);
    p.proofs.submit(&user, &hash);

    let cpu_before = p.env.cpu_instructions();
    let mem_before = p.env.memory_usage();

    // fill -> registry.tier_of + proofs.is_verified are nested cross-contract calls.
    p.settlement.fill(&user, &hash, &100i128);

    assert_cpu_budget(&p.env, cpu_before, 50_000_000);
    assert_mem_budget(&p.env, mem_before, 1_000_000);
}

// ---------------------------------------------------------------------------
// Scenario 16: upgrade of settlement mid-flow preserves fills.
// ---------------------------------------------------------------------------
#[test]
fn scenario_settlement_upgrade_preserves_state() {
    let p = deploy_protocol();
    let user = Address::generate(&p.env);
    p.registry.register(&user, &1u32);

    let hash = sample_hash(&p.env, 16);
    p.proofs.submit(&user, &hash);
    p.settlement.fill(&user, &hash, &100i128);

    let new_wasm = settlement_wasm::WASM;
    p.settlement.upgrade(&new_wasm);

    let fill = p.settlement.fill(&user, &hash, &50i128);
    assert_eq!(fill, 50i128);
}
