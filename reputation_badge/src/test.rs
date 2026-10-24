#![cfg(test)]

//! Integration tests for the `ReputationBadge` prototype (issue #242).
//!
//! Covers the three scenarios called out in the issue's Definition of Done:
//! mint-on-promotion, burn-on-demotion, and a query view proving badge
//! state always matches the last minted/burned tier.
//!
//! Mutation-test hardening (issue #417): the tests below add targeted
//! assertions on auth checks, tier comparisons, and storage arithmetic so
//! surviving mutants in `ReputationBadge` are killed.

use crate::{ReputationBadge, ReputationBadgeClient, Tier};
use soroban_sdk::{testutils::Address as _, Address, Env};

struct Ctx {
    env: Env,
    admin: Address,
    contract_id: Address,
}

impl Ctx {
    fn client(&self) -> ReputationBadgeClient<'_> {
        ReputationBadgeClient::new(&self.env, &self.contract_id)
    }
}

fn setup() -> Ctx {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, ReputationBadge);

    let ctx = Ctx {
        env,
        admin,
        contract_id,
    };
    ctx.client().initialize(&ctx.admin);
    ctx
}

#[test]
fn mint_on_promotion_sets_badge() {
    let ctx = setup();
    let solver = Address::generate(&ctx.env);

    assert_eq!(ctx.client().get_badge(&solver), None);

    ctx.client().mint_badge(&solver, &Tier::Bronze);
    assert_eq!(ctx.client().get_badge(&solver), Some(Tier::Bronze));

    // A further promotion overwrites in place rather than stacking badges.
    ctx.client().mint_badge(&solver, &Tier::Gold);
    assert_eq!(ctx.client().get_badge(&solver), Some(Tier::Gold));
}

#[test]
fn burn_on_demotion_clears_badge() {
    let ctx = setup();
    let solver = Address::generate(&ctx.env);

    ctx.client().mint_badge(&solver, &Tier::Silver);
    assert_eq!(ctx.client().get_badge(&solver), Some(Tier::Silver));

    ctx.client().burn_badge(&solver);
    assert_eq!(ctx.client().get_badge(&solver), None);
}

#[test]
fn get_badge_matches_current_tier_for_unrelated_solvers() {
    let ctx = setup();
    let solver_a = Address::generate(&ctx.env);
    let solver_b = Address::generate(&ctx.env);

    ctx.client().mint_badge(&solver_a, &Tier::Platinum);

    // solver_b never received a badge, and minting for solver_a must not
    // leak state into solver_b's record.
    assert_eq!(ctx.client().get_badge(&solver_a), Some(Tier::Platinum));
    assert_eq!(ctx.client().get_badge(&solver_b), None);
}

#[test]
#[should_panic]
fn burn_badge_without_existing_badge_panics() {
    let ctx = setup();
    let solver = Address::generate(&ctx.env);
    ctx.client().burn_badge(&solver);
}

// --- Mutation-killing tests (issue #417) ---------------------------------

#[test]
fn every_tier_round_trips_through_storage() {
    // Kills mutants that collapse distinct `Tier` variants into one another
    // (e.g. replacing a variant with `Tier::Bronze` or swapping the stored
    // value). Each tier must be observable exactly as written.
    let ctx = setup();
    let solver = Address::generate(&ctx.env);

    for tier in [Tier::Bronze, Tier::Silver, Tier::Gold, Tier::Platinum] {
        ctx.client().mint_badge(&solver, &tier);
        assert_eq!(ctx.client().get_badge(&solver), Some(tier));
    }
}

#[test]
fn mint_requires_admin_auth() {
    // Kills mutants that drop or invert the admin auth check in `mint_badge`.
    let ctx = setup();
    let solver = Address::generate(&ctx.env);

    ctx.client().mint_badge(&solver, &Tier::Gold);

    let auths = ctx.env.auths();
    assert!(
        auths.iter().any(|(addr, _)| addr == &ctx.admin),
        "mint_badge must require the admin's authorization"
    );
}

#[test]
fn burn_requires_admin_auth() {
    // Kills mutants that drop or invert the admin auth check in `burn_badge`.
    let ctx = setup();
    let solver = Address::generate(&ctx.env);

    ctx.client().mint_badge(&solver, &Tier::Silver);
    ctx.client().burn_badge(&solver);

    let auths = ctx.env.auths();
    assert!(
        auths.iter().any(|(addr, _)| addr == &ctx.admin),
        "burn_badge must require the admin's authorization"
    );
}

#[test]
fn burn_only_affects_target_solver() {
    // Kills mutants that remove the wrong key or clear the whole map.
    let ctx = setup();
    let solver_a = Address::generate(&ctx.env);
    let solver_b = Address::generate(&ctx.env);

    ctx.client().mint_badge(&solver_a, &Tier::Gold);
    ctx.client().mint_badge(&solver_b, &Tier::Silver);

    ctx.client().burn_badge(&solver_a);

    assert_eq!(ctx.client().get_badge(&solver_a), None);
    assert_eq!(ctx.client().get_badge(&solver_b), Some(Tier::Silver));
}

#[test]
fn re_mint_after_burn_restores_badge() {
    // Kills mutants that leave a tombstone or skip the write after a burn.
    let ctx = setup();
    let solver = Address::generate(&ctx.env);

    ctx.client().mint_badge(&solver, &Tier::Bronze);
    ctx.client().burn_badge(&solver);
    assert_eq!(ctx.client().get_badge(&solver), None);

    ctx.client().mint_badge(&solver, &Tier::Platinum);
    assert_eq!(ctx.client().get_badge(&solver), Some(Tier::Platinum));
}
