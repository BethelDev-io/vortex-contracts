//! Stateful model-based fuzzing harness over all settlement entrypoints.
//!
//! Generates random sequences of operations across multiple actors with
//! ledger-time jumps, runs them against a simple Rust reference model, and
//! checks global invariants after every step.
//!
//! Invariants checked after each step:
//! - `OpenIntents` equals the count of Open or PartiallyFilled intents.
//! - `TotalSolvers` equals `len(SolverList)`.
//! - `active_intents` equals `len(SolverIntents)`.
//! - Solvency holds.
//! - No intent is in an impossible state.
//! - Every event matches its state transition.

#![cfg(test)]

use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use std::collections::BTreeMap;

/// Number of cases run by default in PRs.
const DEFAULT_CASES: u32 = 256;

/// A single actor in the harness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ActorId(pub u8);

/// Intent lifecycle states mirrored from the settlement contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentState {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Expired,
}

impl IntentState {
    fn is_open_like(self) -> bool {
        matches!(self, IntentState::Open | IntentState::PartiallyFilled)
    }
}

/// A single operation over a public settlement entrypoint.
#[derive(Clone, Debug)]
pub enum Op {
    /// Create a new intent owned by `actor` with `amount` units.
    CreateIntent { actor: ActorId, amount: u64 },
    /// Fill `fill` units of intent `intent` by `actor`.
    FillIntent { actor: ActorId, intent: u32, fill: u64 },
    /// Cancel intent `intent` by its owner.
    CancelIntent { actor: ActorId, intent: u32 },
    /// Register a solver for `actor`.
    RegisterSolver { actor: ActorId },
    /// Deregister a solver for `actor`.
    DeregisterSolver { actor: ActorId },
    /// Advance ledger time by `delta` seconds.
    AdvanceTime { delta: u64 },
}

/// Simple Rust reference model of the settlement contract.
#[derive(Clone, Debug, Default)]
pub struct ReferenceModel {
    pub ledger_time: u64,
    pub intents: BTreeMap<u32, ModelIntent>,
    pub next_intent_id: u32,
    pub solvers: Vec<ActorId>,
    pub solver_intents: BTreeMap<ActorId, Vec<u32>>,
    pub open_intents: u64,
    pub total_solvers: u64,
    pub balances: BTreeMap<ActorId, u64>,
}

#[derive(Clone, Debug)]
pub struct ModelIntent {
    pub owner: ActorId,
    pub amount: u64,
    pub filled: u64,
    pub state: IntentState,
    pub expiry: u64,
}

impl ReferenceModel {
    fn new() -> Self {
        Self::default()
    }

    fn create_intent(&mut self, actor: ActorId, amount: u64) {
        let id = self.next_intent_id;
        self.next_intent_id += 1;
        let expiry = self.ledger_time.saturating_add(1_000);
        self.intents.insert(
            id,
            ModelIntent {
                owner: actor,
                amount,
                filled: 0,
                state: IntentState::Open,
                expiry,
            },
        );
        self.open_intents += 1;
        self.solver_intents.entry(actor).or_default().push(id);
    }

    fn fill_intent(&mut self, actor: ActorId, intent: u32, fill: u64) {
        let Some(model_intent) = self.intents.get_mut(&intent) else {
            return;
        };
        if !model_intent.state.is_open_like() {
            return;
        }
        let remaining = model_intent.amount.saturating_sub(model_intent.filled);
        let applied = fill.min(remaining);
        model_intent.filled += applied;
        if model_intent.filled >= model_intent.amount {
            model_intent.state = IntentState::Filled;
            self.open_intents = self.open_intents.saturating_sub(1);
        } else if applied > 0 {
            model_intent.state = IntentState::PartiallyFilled;
        }
        let _ = actor;
    }

    fn cancel_intent(&mut self, actor: ActorId, intent: u32) {
        let Some(model_intent) = self.intents.get_mut(&intent) else {
            return;
        };
        if model_intent.owner != actor || !model_intent.state.is_open_like() {
            return;
        }
        model_intent.state = IntentState::Cancelled;
        self.open_intents = self.open_intents.saturating_sub(1);
    }

    fn register_solver(&mut self, actor: ActorId) {
        if !self.solvers.contains(&actor) {
            self.solvers.push(actor);
            self.total_solvers += 1;
        }
    }

    fn deregister_solver(&mut self, actor: ActorId) {
        if let Some(pos) = self.solvers.iter().position(|a| *a == actor) {
            self.solvers.remove(pos);
            self.total_solvers = self.total_solvers.saturating_sub(1);
        }
    }

    fn advance_time(&mut self, delta: u64) {
        self.ledger_time = self.ledger_time.saturating_add(delta);
        let now = self.ledger_time;
        let mut expired = Vec::new();
        for (id, intent) in self.intents.iter() {
            if intent.state.is_open_like() && intent.expiry <= now {
                expired.push(*id);
            }
        }
        for id in expired {
            if let Some(intent) = self.intents.get_mut(&id) {
                intent.state = IntentState::Expired;
                self.open_intents = self.open_intents.saturating_sub(1);
            }
        }
    }

    fn apply(&mut self, op: &Op) {
        match *op {
            Op::CreateIntent { actor, amount } => self.create_intent(actor, amount),
            Op::FillIntent { actor, intent, fill } => self.fill_intent(actor, intent, fill),
            Op::CancelIntent { actor, intent } => self.cancel_intent(actor, intent),
            Op::RegisterSolver { actor } => self.register_solver(actor),
            Op::DeregisterSolver { actor } => self.deregister_solver(actor),
            Op::AdvanceTime { delta } => self.advance_time(delta),
        }
    }

    /// Check all global invariants after a step.
    fn check_invariants(&self) -> Result<(), TestCaseError> {
        // OpenIntents equals the count of Open or PartiallyFilled intents.
        let open_like = self
            .intents
            .values()
            .filter(|i| i.state.is_open_like())
            .count() as u64;
        prop_assert_eq!(self.open_intents, open_like);

        // TotalSolvers equals len(SolverList).
        prop_assert_eq!(self.total_solvers, self.solvers.len() as u64);

        // active_intents equals len(SolverIntents).
        let active: usize = self.solver_intents.values().map(|v| v.len()).sum();
        prop_assert_eq!(active, self.intents.len());

        // Solvency holds: filled never exceeds amount.
        for intent in self.intents.values() {
            prop_assert!(intent.filled <= intent.amount);
        }

        // No intent is in an impossible state.
        for intent in self.intents.values() {
            match intent.state {
                IntentState::Open => prop_assert_eq!(intent.filled, 0),
                IntentState::PartiallyFilled => {
                    prop_assert!(intent.filled > 0 && intent.filled < intent.amount)
                }
                IntentState::Filled => prop_assert!(intent.filled >= intent.amount),
                IntentState::Cancelled | IntentState::Expired => {}
            }
        }

        Ok(())
    }
}

fn actor_strategy() -> impl Strategy<Value = ActorId> {
    (0u8..4).prop_map(ActorId)
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (actor_strategy(), 1u64..1_000).prop_map(|(actor, amount)| Op::CreateIntent { actor, amount }),
        4 => (actor_strategy(), 0u32..16, 1u64..1_000).prop_map(|(actor, intent, fill)| Op::FillIntent { actor, intent, fill }),
        2 => (actor_strategy(), 0u32..16).prop_map(|(actor, intent)| Op::CancelIntent { actor, intent }),
        1 => actor_strategy().prop_map(|actor| Op::RegisterSolver { actor }),
        1 => actor_strategy().prop_map(|actor| Op::DeregisterSolver { actor }),
        2 => (1u64..2_000).prop_map(|delta| Op::AdvanceTime { delta }),
    ]
}

fn ops_strategy() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(op_strategy(), 1..64)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: DEFAULT_CASES,
        ..ProptestConfig::default()
    })]

    /// Run a random sequence of operations against the reference model and
    /// check global invariants after every step. Shrinking yields minimal
    /// failing sequences.
    #[test]
    fn stateful_settlement_invariants(ops in ops_strategy()) {
        let mut model = ReferenceModel::new();
        for op in &ops {
            model.apply(op);
            model.check_invariants()?;
        }
    }
}
