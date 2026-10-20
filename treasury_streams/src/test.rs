#![cfg(test)]

extern crate std;

use crate::{TreasuryStreams, TreasuryStreamsClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token, Address, Env,
};

fn set_timestamp(env: &Env, ts: u64) {
    let mut info: LedgerInfo = env.ledger().get();
    info.timestamp = ts;
    env.ledger().set(info);
}

struct Setup {
    env: Env,
    client: TreasuryStreamsClient<'static>,
    token: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();
    set_timestamp(&env, 0);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let token_id = env.register_stellar_asset_contract(admin.clone());
    let token = token::StellarAssetClient::new(&env, &token_id);
    token.mint(&sender, &1_000_000);

    let contract_id = env.register_contract(None, TreasuryStreams);
    let client = TreasuryStreamsClient::new(&env, &contract_id);

    Setup {
        env,
        client,
        token: token_id,
        admin,
        sender,
        recipient,
    }
}

#[test]
fn test_initialize() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);
    // Re-initialization must be rejected.
    let res = s.client.try_initialize(&s.admin, &s.token);
    assert!(res.is_err());
}

#[test]
fn test_create_stream_and_get() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);

    let id = s.client.create_stream(&s.sender, &s.recipient, &1000, &10, &20, &110);
    let stream = s.client.get_stream(&id);
    assert_eq!(stream.total, 1000);
    assert_eq!(stream.start, 10);
    assert_eq!(stream.cliff, 20);
    assert_eq!(stream.end, 110);
    assert_eq!(stream.withdrawn, 0);
    assert_eq!(stream.cancelled, false);

    // Full amount is escrowed up front.
    let token = token::Client::new(&s.env, &s.token);
    assert_eq!(token.balance(&s.sender), 999_000);
}

#[test]
fn test_create_stream_rejects_bad_ordering() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);

    // end <= start
    assert!(s
        .client
        .try_create_stream(&s.sender, &s.recipient, &1000, &100, &100, &100)
        .is_err());
    assert!(s
        .client
        .try_create_stream(&s.sender, &s.recipient, &1000, &100, &100, &50)
        .is_err());

    // cliff before start
    assert!(s
        .client
        .try_create_stream(&s.sender, &s.recipient, &1000, &100, &50, &200)
        .is_err());

    // cliff after end
    assert!(s
        .client
        .try_create_stream(&s.sender, &s.recipient, &1000, &100, &250, &200)
        .is_err());
}

#[test]
fn test_withdraw_pre_cliff() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);
    let id = s.client.create_stream(&s.sender, &s.recipient, &1000, &10, &50, &110);

    set_timestamp(&s.env, 40);
    assert_eq!(s.client.balance_of(&id), 0);
    assert!(s.client.try_withdraw(&id).is_err());
}

#[test]
fn test_withdraw_at_cliff_boundary() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);
    let id = s.client.create_stream(&s.sender, &s.recipient, &1000, &10, &50, &110);

    // Exactly at cliff: vested = total * (cliff - start) / (end - start)
    set_timestamp(&s.env, 50);
    let expected = 1000 * (50 - 10) / (110 - 10);
    assert_eq!(s.client.balance_of(&id), expected);
    s.client.withdraw(&id);
    assert_eq!(s.client.get_stream(&id).withdrawn, expected);
}

#[test]
fn test_withdraw_mid_vesting() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);
    let id = s.client.create_stream(&s.sender, &s.recipient, &1000, &10, &20, &110);

    set_timestamp(&s.env, 60);
    let expected = 1000 * (60 - 10) / (110 - 10);
    assert_eq!(s.client.balance_of(&id), expected);
    s.client.withdraw(&id);

    let token = token::Client::new(&s.env, &s.token);
    assert_eq!(token.balance(&s.recipient), expected);
    assert_eq!(s.client.get_stream(&id).withdrawn, expected);
}

#[test]
fn test_withdraw_at_end_boundary() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);
    let id = s.client.create_stream(&s.sender, &s.recipient, &1000, &10, &20, &110);

    set_timestamp(&s.env, 110);
    assert_eq!(s.client.balance_of(&id), 1000);
    s.client.withdraw(&id);

    let token = token::Client::new(&s.env, &s.token);
    assert_eq!(token.balance(&s.recipient), 1000);
}

#[test]
fn test_withdraw_post_end() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);
    let id = s.client.create_stream(&s.sender, &s.recipient, &1000, &10, &20, &110);

    set_timestamp(&s.env, 500);
    assert_eq!(s.client.balance_of(&id), 1000);
    s.client.withdraw(&id);

    let token = token::Client::new(&s.env, &s.token);
    assert_eq!(token.balance(&s.recipient), 1000);
    // Nothing left to withdraw.
    assert!(s.client.try_withdraw(&id).is_err());
}

#[test]
fn test_cancel_splits_vested_and_unvested() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);
    let id = s.client.create_stream(&s.sender, &s.recipient, &1000, &10, &20, &110);

    set_timestamp(&s.env, 60);
    let vested = 1000 * (60 - 10) / (110 - 10);
    s.client.cancel(&id);

    let token = token::Client::new(&s.env, &s.token);
    // Recipient receives vested portion, sender refunded the rest.
    assert_eq!(token.balance(&s.recipient), vested);
    assert_eq!(token.balance(&s.sender), 999_000 + (1000 - vested));
    assert_eq!(s.client.get_stream(&id).cancelled, true);
}

#[test]
fn test_cancel_twice_rejected() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);
    let id = s.client.create_stream(&s.sender, &s.recipient, &1000, &10, &20, &110);

    set_timestamp(&s.env, 60);
    s.client.cancel(&id);
    assert!(s.client.try_cancel(&id).is_err());
}

#[test]
fn test_large_total_long_duration() {
    let s = setup();
    s.client.initialize(&s.admin, &s.token);

    // Large total over a long duration to probe vested_amount arithmetic.
    let total: i128 = 1_000_000_000_000_000_000;
    let token = token::StellarAssetClient::new(&s.env, &s.token);
    token.mint(&s.sender, &total);

    let id = s.client.create_stream(&s.sender, &s.recipient, &total, &0, &0, &1_000_000_000);
    set_timestamp(&s.env, 500_000_000);

    let expected = total * 500_000_000 / 1_000_000_000;
    assert_eq!(s.client.balance_of(&id), expected);
}
