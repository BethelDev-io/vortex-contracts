#![no_std]

//! Vortex Protocol — Solver Reputation Tier Badge (prototype)
//!
//! Minimal, non-transferable on-chain record of a solver's reputation tier,
//! prototyping the roadmap item tracked by issue #242. See
//! `docs/242-reputation-tier-badge-design.md` for the design rationale
//! (soulbound vs. transferable, bespoke contract vs. SEP-41 token).
//!
//! `mint_badge` / `burn_badge` are meant to be driven by `solver_registry`'s
//! tier-computation logic (issue #1) whenever a solver's tier changes.
//! Until that contract exists, both entry points are gated behind
//! `require_admin` as a placeholder authority.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, panic_with_error, Address, Env, Symbol};

#[cfg(test)]
mod test;

#[cfg(test)]
mod bench;

// ─── TTL Constants ────────────────────────────────────────────────────────────
//
// Follows the convention documented in `docs/ttl-constants-rationale.md` and
// mirrored from `backstop_vault`'s TTL helpers.

/// Bump the instance TTL when it drops below this many ledgers.
const INSTANCE_TTL_THRESHOLD: u32 = 17_280; // ~1 day at 5s/ledger
/// Extend the instance TTL to this many ledgers.
const INSTANCE_TTL_EXTEND_TO: u32 = 518_400; // ~30 days at 5s/ledger

/// Bump a persistent entry's TTL when it drops below this many ledgers.
const PERSISTENT_TTL_THRESHOLD: u32 = 17_280; // ~1 day at 5s/ledger
/// Extend a persistent entry's TTL to this many ledgers.
const PERSISTENT_TTL_EXTEND_TO: u32 = 518_400; // ~30 days at 5s/ledger

// ─── Storage Keys ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum BadgeKey {
    /// Admin address (set in `initialize`). Placeholder authority for
    /// `mint_badge`/`burn_badge` until `solver_registry` (issue #1) exists.
    Admin,
    /// A solver's current tier badge, if any. Absence means `Unranked`.
    Badge(Address),
}

/// Reputation tiers, matching `docs/solver-registry-design.md` §3.
/// `Unranked` is intentionally not representable here — it is modeled as
/// the *absence* of a `BadgeKey::Badge` entry, so a badge's mere presence
/// is proof of `Bronze`-or-above standing.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
    Platinum,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    /// `initialize` called on an already-initialized contract.
    AlreadyInitialized = 1,
    /// Caller is not the admin.
    Unauthorized = 2,
    /// Contract not initialized (`Admin` key absent).
    NotInitialized = 3,
    /// `burn_badge` called for a solver with no badge on record.
    BadgeNotFound = 4,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct ReputationBadge;

#[contractimpl]
impl ReputationBadge {
    /// Deploy-time setup. Must be called exactly once.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&BadgeKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&BadgeKey::Admin, &admin);
        Self::bump_instance_ttl(&env);
    }

    /// Mint (or overwrite, on a tier change) `solver`'s badge to `tier`.
    /// Overwriting in place means a tier upgrade or downgrade never leaves
    /// a stale old-tier badge for a caller to mistakenly read.
    pub fn mint_badge(env: Env, solver: Address, tier: Tier) {
        Self::require_admin(&env);
        let key = BadgeKey::Badge(solver.clone());
        env.storage().persistent().set(&key, &tier);
        Self::bump_persistent_ttl(&env, &key);
        env.events()
            .publish((Symbol::new(&env, "badge_minted"), solver), tier);
    }

    /// Burn `solver`'s badge (used when a solver drops below the `Bronze`
    /// threshold to `Unranked`). Errors if the solver has no badge.
    pub fn burn_badge(env: Env, solver: Address) {
        Self::require_admin(&env);
        if !env
            .storage()
            .persistent()
            .has(&BadgeKey::Badge(solver.clone()))
        {
            panic_with_error!(&env, Error::BadgeNotFound);
        }
        env.storage()
            .persistent()
            .remove(&BadgeKey::Badge(solver.clone()));
        env.events()
            .publish((Symbol::new(&env, "badge_burned"), solver), ());
    }

    /// Read-only: `solver`'s current tier badge, or `None` if `Unranked`.
    pub fn get_badge(env: Env, solver: Address) -> Option<Tier> {
        env.storage().persistent().get(&BadgeKey::Badge(solver))
    }

    fn require_admin(env: &Env) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&BadgeKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
        admin.require_auth();
    }

    /// Extend the instance storage TTL if it has fallen below the threshold.
    fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
    }

    /// Extend a persistent entry's TTL if it has fallen below the threshold.
    fn bump_persistent_ttl(env: &Env, key: &BadgeKey) {
        env.storage().persistent().extend_ttl(
            key,
            PERSISTENT_TTL_THRESHOLD,
            PERSISTENT_TTL_EXTEND_TO,
        );
    }
}
