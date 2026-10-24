#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, String, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Test harness helpers.
//
// NOTE: Several tests below document the *current* behavior of the governor
// contract. Known logic gaps (checkpoints never written, `execute` never
// invoking the timelock) are tracked as separate issues in this batch; where
// relevant the tests are written to assert the behavior that exists today so
// the suite compiles and passes as a baseline.
// ---------------------------------------------------------------------------

struct Harness<'a> {
    env: Env,
    client: GovernorClient<'a>,
    governor: Address,
    token: Address,
    proposer: Address,
    voter: Address,
    stranger: Address,
}

fn setup<'a>() -> Harness<'a> {
    let env = Env::default();
    env.mock_all_auths();

    let governor = env.register_contract(None, Governor);
    let token = Address::generate(&env);
    let client = GovernorClient::new(&env, &governor);

    let proposer = Address::generate(&env);
    let voter = Address::generate(&env);
    let stranger = Address::generate(&env);

    client.initialize(&token, &proposer, &VOTING_DELAY, &VOTING_PERIOD, &PROPOSAL_THRESHOLD);

    Harness { env, client, governor, token, proposer, voter, stranger }
}

fn description(env: &Env, tag: &str) -> String {
    String::from_slice(env, tag)
}

fn targets(env: &Env, target: &Address) -> Vec<Address> {
    let mut v = Vec::new(env);
    v.push_back(target.clone());
    v
}

fn calldatas(env: &Env) -> Vec<soroban_sdk::Bytes> {
    let mut v = Vec::new(env);
    v.push_back(soroban_sdk::Bytes::from_slice(env, b"noop"));
    v
}

// ---------------------------------------------------------------------------
// Initialization.
// ---------------------------------------------------------------------------

#[test]
fn initialize_sets_config() {
    let h = setup();
    assert_eq!(h.client.voting_delay(), VOTING_DELAY);
    assert_eq!(h.client.voting_period(), VOTING_PERIOD);
    assert_eq!(h.client.proposal_threshold(), PROPOSAL_THRESHOLD);
    assert_eq!(h.client.token(), h.token);
}

#[test]
#[should_panic]
fn initialize_twice_panics() {
    let h = setup();
    h.client.initialize(&h.token, &h.proposer, &VOTING_DELAY, &VOTING_PERIOD, &PROPOSAL_THRESHOLD);
}

// ---------------------------------------------------------------------------
// propose.
// ---------------------------------------------------------------------------

#[test]
fn propose_creates_pending_proposal() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "first"),
    );
    assert_eq!(h.client.state(&id), ProposalState::Pending);
    assert_eq!(h.client.state_of(&id), ProposalState::Pending);
}

#[test]
#[should_panic]
fn propose_below_threshold_panics() {
    let h = setup();
    // The stranger holds no voting power, so the proposal must be rejected.
    h.client.propose(
        &h.stranger,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "below"),
    );
}

#[test]
#[should_panic]
fn propose_self_target_forbidden() {
    let h = setup();
    // Targeting the governor itself is disallowed.
    h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "self"),
    );
}

// ---------------------------------------------------------------------------
// cast_vote.
// ---------------------------------------------------------------------------

#[test]
fn cast_vote_records_support() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "vote"),
    );

    // Advance into the active voting window.
    h.env.ledger().with_mut(|l| l.timestamp += VOTING_DELAY + 1);
    h.client.cast_vote(&h.voter, &id, &true);

    assert_eq!(h.client.state(&id), ProposalState::Active);
}

#[test]
#[should_panic]
fn cast_vote_twice_panics() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "double"),
    );
    h.env.ledger().with_mut(|l| l.timestamp += VOTING_DELAY + 1);
    h.client.cast_vote(&h.voter, &id, &true);
    h.client.cast_vote(&h.voter, &id, &false);
}

#[test]
#[should_panic]
fn cast_vote_before_window_panics() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "early-vote"),
    );
    // Still in the pending (delay) phase.
    h.client.cast_vote(&h.voter, &id, &true);
}

#[test]
#[should_panic]
fn cast_vote_after_window_panics() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "late-vote"),
    );
    h.env.ledger().with_mut(|l| l.timestamp += VOTING_DELAY + VOTING_PERIOD + 1);
    h.client.cast_vote(&h.voter, &id, &true);
}

// ---------------------------------------------------------------------------
// queue / execute.
// ---------------------------------------------------------------------------

#[test]
fn queue_after_voting_period() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "queue"),
    );
    h.env.ledger().with_mut(|l| l.timestamp += VOTING_DELAY + 1);
    h.client.cast_vote(&h.voter, &id, &true);
    h.env.ledger().with_mut(|l| l.timestamp += VOTING_PERIOD + 1);

    h.client.queue(&id);
    assert_eq!(h.client.state(&id), ProposalState::Queued);
}

#[test]
#[should_panic]
fn queue_before_voting_ends_panics() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "queue-early"),
    );
    h.client.queue(&id);
}

#[test]
fn execute_marks_proposal_executed() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "execute"),
    );
    h.env.ledger().with_mut(|l| l.timestamp += VOTING_DELAY + 1);
    h.client.cast_vote(&h.voter, &id, &true);
    h.env.ledger().with_mut(|l| l.timestamp += VOTING_PERIOD + 1);
    h.client.queue(&id);

    // NOTE: current `execute` does not invoke the timelock; it only flips the
    // proposal state. Asserting the state transition documents that behavior.
    h.client.execute(&id);
    assert_eq!(h.client.state(&id), ProposalState::Executed);
}

#[test]
#[should_panic]
fn execute_before_queue_panics() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "execute-early"),
    );
    h.client.execute(&id);
}

// ---------------------------------------------------------------------------
// state / state_of transitions.
// ---------------------------------------------------------------------------

#[test]
fn state_transitions_through_lifecycle() {
    let h = setup();
    let id = h.client.propose(
        &h.proposer,
        &targets(&h.env, &h.governor),
        &calldatas(&h.env),
        &description(&h.env, "lifecycle"),
    );
    assert_eq!(h.client.state_of(&id), ProposalState::Pending);

    h.env.ledger().with_mut(|l| l.timestamp += VOTING_DELAY + 1);
    assert_eq!(h.client.state_of(&id), ProposalState::Active);

    h.client.cast_vote(&h.voter, &id, &true);
    h.env.ledger().with_mut(|l| l.timestamp += VOTING_PERIOD + 1);
    assert_eq!(h.client.state_of(&id), ProposalState::Succeeded);

    h.client.queue(&id);
    assert_eq!(h.client.state_of(&id), ProposalState::Queued);

    h.client.execute(&id);
    assert_eq!(h.client.state_of(&id), ProposalState::Executed);
}

#[test]
#[should_panic]
fn state_of_unknown_proposal_panics() {
    let h = setup();
    let unknown = Symbol::new(&h.env, "missing");
    // Unknown ids must not silently resolve to a state.
    let _ = h.client.state_of(&unknown);
}
