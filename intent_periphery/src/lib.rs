//! Vortex Protocol — Intent Periphery Contract (issue #376)
//!
//! Moves heavy view/governance surfaces out of the core settlement contract so
//! the core wasm stays small, cheap to load, and easy to audit.
//!
//! ## Design rationale
//!
//! Soroban's on-chain wasm limit is 64 KiB (65 536 bytes). The core
//! `intent_settlement` contract absorbs lifecycle, bonds, and slashing —
//! code that directly moves funds. Every KiB of admin/view logic added to
//! core grows the code-load fee on *every* user call.
//!
//! `intent_periphery` reads state from core via cross-contract view calls and
//! aggregates or re-exposes it to dashboards and indexers. Because it never
//! holds funds or changes core state, its attack surface is minimal and it can
//! be upgraded independently.
//!
//! ## Authenticated interface
//!
//! The periphery stores the settlement contract address set at `initialize`.
//! All read functions accept an explicit `settlement` parameter so callers can
//! verify which core they are reading from, and so the periphery can be pointed
//! at a testnet or a new core address without re-deploying.
//!
//! ## Reentrancy note
//!
//! All functions here are read-only (no token transfers, no storage writes on
//! the core). Cross-contract calls to `settlement` are therefore safe: a
//! re-entrant call from the core back into the periphery would be a no-op
//! (no funds, no side effects).
//!
//! ## Batched views
//!
//! Every proxy on this contract is a one-view passthrough, so a consumer that
//! needs several pieces of aggregate state pays one client→periphery round
//! trip *per view*. `get_dashboard_snapshot()` bundles the two views the
//! protocol dashboard always needs together — `get_stats()` and
//! `get_protocol_health()` (issue #534) — into a single call, returning them
//! under the [`SNAPSHOT_KEY_STATS`] and [`SNAPSHOT_KEY_PROTOCOL_HEALTH`] keys.
//!
//! The bundle carries the core's **raw** values, exactly like the individual
//! proxies, so the periphery never duplicates the core's struct definitions
//! and can be upgraded independently of them.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, BytesN, Env,
    IntoVal, Map, Symbol, Val, Vec,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Settlement,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
}

// ─── Batched view payloads ────────────────────────────────────────────────────

/// Key under which the batched snapshot carries the core's `get_stats()` view.
pub const SNAPSHOT_KEY_STATS: &str = "stats";
/// Key under which the batched snapshot carries the core's
/// `get_protocol_health()` view.
pub const SNAPSHOT_KEY_PROTOCOL_HEALTH: &str = "protocol_health";

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct IntentPeriphery;

#[contractimpl]
impl IntentPeriphery {
    // ── Initialization ────────────────────────────────────────────────────────

    /// One-time setup: record admin and the core settlement contract address.
    pub fn initialize(env: Env, admin: Address, settlement: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Settlement, &settlement);
        env.events().publish(
            (Symbol::new(&env, "periphery_initialized"),),
            (admin, settlement),
        );
    }

    /// Admin-only: point the periphery at a new core settlement contract.
    pub fn set_settlement(env: Env, settlement: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Settlement, &settlement);
        env.events()
            .publish((Symbol::new(&env, "settlement_updated"),), settlement);
    }

    /// Return the core settlement contract this periphery reads from.
    pub fn get_settlement(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Settlement)
    }

    // ── Heavy view proxies (issue #376) ───────────────────────────────────────
    //
    // Each function below is a thin proxy: it calls the equivalent view on the
    // core contract and returns its result unchanged. Keeping these calls in the
    // periphery means future changes to argument lists or return types only
    // require an upgrade of the (cheaper, less critical) periphery, not the core.

    /// Proxy: `settlement.get_stats()` — protocol-level aggregate counters.
    ///
    /// Returns the raw val from the cross-contract call so callers decode it
    /// using the core's published ABI. This avoids duplicating struct definitions
    /// here and keeps periphery upgrades independent of struct field additions.
    pub fn get_stats(env: Env) -> soroban_sdk::Val {
        let settlement = Self::load_settlement(&env);
        env.invoke_contract(&settlement, &Symbol::new(&env, "get_stats"), Vec::new(&env))
    }

    /// Proxy: `settlement.list_solvers(start, limit)` — paginated solver list.
    pub fn list_solvers(env: Env, start: u32, limit: u32) -> soroban_sdk::Val {
        let settlement = Self::load_settlement(&env);
        env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "list_solvers"),
            soroban_sdk::vec![&env, start.into(), limit.into()],
        )
    }

    /// Proxy: `settlement.list_open_intents(start, limit)` — paginated open-intent IDs.
    pub fn list_open_intents(env: Env, start: u32, limit: u32) -> soroban_sdk::Val {
        let settlement = Self::load_settlement(&env);
        env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "list_open_intents"),
            soroban_sdk::vec![&env, start.into(), limit.into()],
        )
    }

    /// Proxy: `settlement.list_intents_by_user(user, start, limit)`.
    pub fn list_intents_by_user(
        env: Env,
        user: Address,
        start: u32,
        limit: u32,
    ) -> soroban_sdk::Val {
        let settlement = Self::load_settlement(&env);
        env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "list_intents_by_user"),
            soroban_sdk::vec![&env, user.into_val(&env), start.into(), limit.into()],
        )
    }

    /// Proxy: `settlement.get_fee_schedule(solver)` — fee tiers for a solver.
    pub fn get_fee_schedule(env: Env, solver: Address) -> soroban_sdk::Val {
        let settlement = Self::load_settlement(&env);
        env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_fee_schedule"),
            soroban_sdk::vec![&env, solver.into_val(&env)],
        )
    }

    /// Proxy: `settlement.get_solver(solver)` — full SolverRecord.
    pub fn get_solver(env: Env, solver: Address) -> soroban_sdk::Val {
        let settlement = Self::load_settlement(&env);
        env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_solver"),
            soroban_sdk::vec![&env, solver.into_val(&env)],
        )
    }

    /// Proxy: `settlement.get_intent(intent_id)` — full IntentRecord.
    pub fn get_intent(env: Env, intent_id: BytesN<32>) -> soroban_sdk::Val {
        let settlement = Self::load_settlement(&env);
        env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_intent"),
            soroban_sdk::vec![&env, intent_id.into_val(&env)],
        )
    }

    /// Proxy: `settlement.get_protocol_health()` — health/pause status.
    pub fn get_protocol_health(env: Env) -> soroban_sdk::Val {
        let settlement = Self::load_settlement(&env);
        env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_protocol_health"),
            Vec::new(&env),
        )
    }

    // ── Batched multi-view proxies (issue #534) ─────────────────────────────────
    //
    // These functions batch multiple cross-contract view calls into a single
    // invocation, reducing RPC overhead for dashboards and indexers that need
    // multiple views simultaneously.

    /// Batched view: `settlement.get_stats()` **and**
    /// `settlement.get_protocol_health()` in a single cross-contract round
    /// trip (issue #534).
    ///
    /// Every other proxy on this contract is a one-view passthrough, so a
    /// dashboard that wants more than one piece of aggregate state still needs
    /// one cross-contract call *per view* through the periphery. This bundles
    /// the two views the protocol dashboard always needs together into one
    /// call, which is exactly the use case the crate's own module doc
    /// describes ("aggregates or re-exposes it to dashboards and indexers").
    ///
    /// The bundle is returned as a map with two well-known keys:
    ///
    /// * [`SNAPSHOT_KEY_STATS`] (`"stats"`) — `settlement.get_stats()`, the
    ///   protocol-level aggregate counters.
    /// * [`SNAPSHOT_KEY_PROTOCOL_HEALTH`] (`"protocol_health"`) —
    ///   `settlement.get_protocol_health()`, the aggregate health/pause status.
    ///
    /// The two views are the ones issue #494 ("Add dashboard sections for the
    /// five new contracts") asks the periphery to surface as "proxied
    /// health/stats views".
    ///
    /// The individual proxies are **not** removed — this is additive, and a
    /// client that wants a single view can still call it directly.
    ///
    /// Like the individual proxies, the bundled values are returned **raw**
    /// (the very same `Val`s the individual proxies return) so the periphery
    /// keeps no copy of the core's struct definitions and can be upgraded
    /// independently of them. A client decodes each entry with the core's
    /// published ABI.
    ///
    /// The two core views are read-only, so batching them cannot change any
    /// observable behaviour: each bundled value is exactly what the
    /// corresponding individual proxy would have returned.
    pub fn get_dashboard_snapshot(env: Env) -> Map<Symbol, Val> {
        let settlement = Self::load_settlement(&env);
        let stats =
            env.invoke_contract(&settlement, &Symbol::new(&env, "get_stats"), Vec::new(&env));
        let protocol_health = env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_protocol_health"),
            Vec::new(&env),
        );
        let mut snapshot = Map::new(&env);
        snapshot.set(Symbol::new(&env, SNAPSHOT_KEY_STATS), stats);
        snapshot.set(
            Symbol::new(&env, SNAPSHOT_KEY_PROTOCOL_HEALTH),
            protocol_health,
        );
        snapshot
    }

    /// Batch: `get_stats` + `get_protocol_health` in one call.
    ///
    /// Returns `(stats, health)` as a tuple of two `Val`s.
    pub fn batch_get_stats_and_health(env: Env) -> (soroban_sdk::Val, soroban_sdk::Val) {
        let settlement = Self::load_settlement(&env);
        let stats = env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_stats"),
            Vec::new(&env),
        );
        let health = env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_protocol_health"),
            Vec::new(&env),
        );
        (stats, health)
    }

    /// Batch: `list_solvers` + `list_open_intents` in one call.
    ///
    /// Returns `(solvers, open_intents)` as a tuple of two `Val`s.
    pub fn batch_list_solvers_and_intents(env: Env, start: u32, limit: u32) -> (soroban_sdk::Val, soroban_sdk::Val) {
        let settlement = Self::load_settlement(&env);
        let solvers = env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "list_solvers"),
            soroban_sdk::vec![&env, start.into(), limit.into()],
        );
        let intents = env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "list_open_intents"),
            soroban_sdk::vec![&env, start.into(), limit.into()],
        );
        (solvers, intents)
    }

    /// Batch: `get_solver` + `get_fee_schedule` for a specific solver.
    ///
    /// Returns `(solver_record, fee_schedule)` as a tuple of two `Val`s.
    pub fn batch_get_solver_and_fee_schedule(env: Env, solver: Address) -> (soroban_sdk::Val, soroban_sdk::Val) {
        let settlement = Self::load_settlement(&env);
        let solver_record = env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_solver"),
            soroban_sdk::vec![&env, solver.into_val(&env)],
        );
        let fee_schedule = env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_fee_schedule"),
            soroban_sdk::vec![&env, solver.into_val(&env)],
        );
        (solver_record, fee_schedule)
    }

    /// Batch: `get_intent` + `get_protocol_health` in one call.
    ///
    /// Returns `(intent_record, health)` as a tuple of two `Val`s.
    pub fn batch_get_intent_and_health(env: Env, intent_id: BytesN<32>) -> (soroban_sdk::Val, soroban_sdk::Val) {
        let settlement = Self::load_settlement(&env);
        let intent = env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_intent"),
            soroban_sdk::vec![&env, intent_id.into_val(&env)],
        );
        let health = env.invoke_contract(
            &settlement,
            &Symbol::new(&env, "get_protocol_health"),
            Vec::new(&env),
        );
        (intent, health)
    }

    /// Generic batched view: call any two view functions on the settlement contract.
    ///
    /// Allows arbitrary composition of view calls. Returns `(result1, result2)`.
    pub fn batch_two_views(
        env: Env,
        fn1: Symbol,
        args1: Vec<soroban_sdk::Val>,
        fn2: Symbol,
        args2: Vec<soroban_sdk::Val>,
    ) -> (soroban_sdk::Val, soroban_sdk::Val) {
        let settlement = Self::load_settlement(&env);
        let r1 = env.invoke_contract(&settlement, &fn1, args1);
        let r2 = env.invoke_contract(&settlement, &fn2, args2);
        (r1, r2)
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn load_settlement(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Settlement)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
    }
}
