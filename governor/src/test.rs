//! Tests for the Governor contract.

use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol, Vec};

fn setup() -> (Env, GovernorConfig) {
    let env = Env::default();
    env.mock_all_auths();

    let registry = Address::generate(&env);
    let timelock = Address::generate(&env);
    let proposer = Address::generate(&env);

    let config = GovernorConfig {
        registry,
        timelock,
        voting_delay: 10,
        voting_period: 100,
        proposal_threshold: 100,
        quorum: 50,
    };

    Governor::initialize(env.clone(), config.clone()).unwrap();

    (env, config)
}

#[test]
fn test_propose_below_threshold() {
    let (env, config) = setup();
    let proposer = Address::generate(&env);

    // Power is 0 (no checkpoint), below threshold of 100
    let result = Governor::propose(env, proposer, Address::generate(&env), Symbol::new(&env, "set_fee"), soroban_sdk::BytesN::<32>::from_array(&env, &[0; 32]));
    assert_eq!(result, Err(GovernorError::BelowProposalThreshold));
}

#[test]
fn test_cast_vote_before_voting_delay() {
    let (env, config) = setup();
    let proposer = Address::generate(&env);

    // Give proposer voting power by setting a checkpoint
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        proposer.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 200 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    let id = Governor::propose(env.clone(), proposer, Address::generate(&env), Symbol::new(&env, "set_fee"), soroban_sdk::BytesN::<32>::from_array(&env, &[0; 32])).unwrap();

    // Try to vote immediately (before voting_delay = 10)
    let voter = Address::generate(&env);
    let result = Governor::cast_vote(env, voter, id, VoteType::For);
    assert_eq!(result, Err(GovernorError::NotActive));
}

#[test]
fn test_cast_vote_after_deadline_boundary() {
    let (env, config) = setup();
    let proposer = Address::generate(&env);

    // Give proposer voting power
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        proposer.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 200 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    let id = Governor::propose(env.clone(), proposer, Address::generate(&env), Symbol::new(&env, "set_fee"), soroban_sdk::BytesN::<32>::from_array(&env, &[0; 32])).unwrap();

    // snapshot = 5, voting_delay = 10, voting_period = 100
    // deadline = 5 + 10 + 100 = 115
    // voting starts at 15, ends at 115

    // Advance to deadline - 1 (114) - should work
    env.ledger().set_sequence(114);
    let voter = Address::generate(&env);
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        voter.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 100 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    let result = Governor::cast_vote(env.clone(), voter, id, VoteType::For);
    assert!(result.is_ok());

    // Advance to deadline (115) - should still work (deadline is inclusive)
    let voter2 = Address::generate(&env);
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        voter2.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 100 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    env.ledger().set_sequence(115);
    let result = Governor::cast_vote(env.clone(), voter2, id, VoteType::Against);
    assert!(result.is_ok());

    // Advance to deadline + 1 (116) - should fail (voting closed)
    let voter3 = Address::generate(&env);
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        voter3.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 100 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    env.ledger().set_sequence(116);
    let result = Governor::cast_vote(env, voter3, id, VoteType::Abstain);
    assert_eq!(result, Err(GovernorError::VotingClosed));
}

#[test]
fn test_cast_vote_at_exact_voting_delay_start() {
    let (env, config) = setup();
    let proposer = Address::generate(&env);

    // Give proposer voting power
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        proposer.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 200 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    let id = Governor::propose(env.clone(), proposer, Address::generate(&env), Symbol::new(&env, "set_fee"), soroban_sdk::BytesN::<32>::from_array(&env, &[0; 32])).unwrap();

    // snapshot = 5, voting_delay = 10
    // voting starts at ledger 15 (inclusive)
    env.ledger().set_sequence(15);

    let voter = Address::generate(&env);
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        voter.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 100 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    let result = Governor::cast_vote(env, voter, id, VoteType::For);
    assert!(result.is_ok());
}

#[test]
fn test_cast_vote_at_exact_deadline_inclusive() {
    let (env, config) = setup();
    let proposer = Address::generate(&env);

    // Give proposer voting power
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        proposer.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 200 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    let id = Governor::propose(env.clone(), proposer, Address::generate(&env), Symbol::new(&env, "set_fee"), soroban_sdk::BytesN::<32>::from_array(&env, &[0; 32])).unwrap();

    // deadline = 5 + 10 + 100 = 115
    // voting is allowed at ledger 115 (inclusive)
    env.ledger().set_sequence(115);

    let voter = Address::generate(&env);
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        voter.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 100 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    let result = Governor::cast_vote(env, voter, id, VoteType::For);
    assert!(result.is_ok());
}

#[test]
fn test_cast_vote_after_deadline_exclusive() {
    let (env, config) = setup();
    let proposer = Address::generate(&env);

    // Give proposer voting power
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        proposer.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 200 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    let id = Governor::propose(env.clone(), proposer, Address::generate(&env), Symbol::new(&env, "set_fee"), soroban_sdk::BytesN::<32>::from_array(&env, &[0; 32])).unwrap();

    // deadline = 115, voting closed at 116
    env.ledger().set_sequence(116);

    let voter = Address::generate(&env);
    let checkpoint_key = (
        Symbol::new(&env, "checkpoint"),
        config.registry.clone(),
        voter.clone(),
    );
    let checkpoints = Vec::from_array(&env, &[Checkpoint { ledger: 5, power: 100 }]);
    env.storage().persistent().set(&checkpoint_key, &checkpoints);

    let result = Governor::cast_vote(env, voter, id, VoteType::For);
    assert_eq!(result, Err(GovernorError::VotingClosed));
}