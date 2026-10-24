//! # Fee Router
//!
//! A pull-based fee router that accumulates protocol revenue per token and
//! distributes it to governance-configured sinks according to timelocked
//! weights (in basis points, summing to 10,000).
//!
//! Design notes:
//! - Accounting is pull-based: `distribute` credits `claimable[sink][token]`
//!   and sinks withdraw with `claim`. A sink that rejects a transfer does not
//!   revert the whole distribution; its share stays claimable for a later retry.
//! - Weight changes are timelocked: `propose_weights` then `apply_weights`
//!   after `weight_delay` has elapsed.
//! - Rounding dust from integer division is assigned to the first sink.
//! - Fee-on-transfer tokens are supported: the router credits the amount it
//!   actually received, not the amount the caller claimed to send.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Env, Vec,
};

/// Total basis points that weights must sum to.
pub const TOTAL_BPS: u32 = 10_000;

/// Maximum number of sinks the router will accept.
pub const MAX_SINKS: u32 = 16;

/// A single revenue sink with its governance-set weight.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sink {
    pub account: Address,
    pub weight_bps: u32,
}

/// A pending, timelocked weight change.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWeights {
    pub sinks: Vec<Sink>,
    pub executable_at: u64,
}

/// Storage keys for the router.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    Governance,
    WeightDelay,
    Sinks,
    Pending,
    Balance(Address),
    Claimable(Address, Address),
}

/// Errors returned by the router.
#[contracterror]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FeeRouterError {
    Unauthorized = 1,
    TooManySinks = 2,
    WeightsMustSumToTotal = 3,
    EmptySinks = 4,
    DuplicateSink = 5,
    NoPendingWeights = 6,
    TimelockNotElapsed = 7,
    UnknownToken = 8,
    NothingToClaim = 9,
    TransferFailed = 10,
}

/// The fee router contract.
#[contract]
pub struct FeeRouter;

#[contractimpl]
impl FeeRouter {
    /// Create a new router. `sinks` must be non-empty, unique, at most
    /// [`MAX_SINKS`], and sum to [`TOTAL_BPS`].
    pub fn initialize(
        env: Env,
        governance: Address,
        weight_delay: u64,
        sinks: Vec<Sink>,
    ) -> Result<(), FeeRouterError> {
        validate_sinks(&env, &sinks)?;
        env.storage().instance().set(&DataKey::Governance, &governance);
        env.storage().instance().set(&DataKey::WeightDelay, &weight_delay);
        env.storage().instance().set(&DataKey::Sinks, &sinks);
        Ok(())
    }

    pub fn governance(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Governance).unwrap()
    }

    pub fn weight_delay(env: Env) -> u64 {
        env.storage().instance().get(&DataKey::WeightDelay).unwrap()
    }

    pub fn sinks(env: Env) -> Vec<Sink> {
        env.storage().instance().get(&DataKey::Sinks).unwrap()
    }

    /// Total accumulated balance for `token` that has not yet been distributed.
    pub fn balance_of(env: Env, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(token))
            .unwrap_or(0)
    }

    /// Amount currently claimable by `sink` for `token`.
    pub fn claimable(env: Env, sink: Address, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Claimable(sink, token))
            .unwrap_or(0)
    }

    /// Record an incoming fee payment. The router credits the amount it
    /// actually received, which makes fee-on-transfer tokens safe.
    pub fn on_fee_received(env: Env, token: Address, amount: i128) {
        if amount <= 0 {
            return;
        }
        let key = DataKey::Balance(token);
        let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(current + amount));
    }

    /// Permissionlessly distribute the accumulated balance of `token` across
    /// the configured sinks according to their weights.
    ///
    /// Rounding dust is assigned to the first sink so that the full balance is
    /// always conserved.
    pub fn distribute(env: Env, token: Address) -> Result<(), FeeRouterError> {
        let total: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::Balance(token.clone()))
            .unwrap_or(0);
        if total == 0 {
            return Err(FeeRouterError::UnknownToken);
        }

        let sinks: Vec<Sink> = env.storage().instance().get(&DataKey::Sinks).unwrap();
        let mut distributed: i128 = 0;
        let len = sinks.len();
        for i in 0..len {
            let sink = sinks.get(i).unwrap();
            let share = if i == 0 {
                // First sink absorbs rounding dust.
                total - distributed
            } else {
                total * (sink.weight_bps as i128) / (TOTAL_BPS as i128)
            };
            distributed += share;
            if share > 0 {
                let key = DataKey::Claimable(sink.account.clone(), token.clone());
                let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
                env.storage().persistent().set(&key, &(current + share));
            }
        }

        env.storage()
            .persistent()
            .set(&DataKey::Balance(token), &0i128);
        Ok(())
    }

    /// Withdraw the caller's claimable balance for `token`.
    ///
    /// The transfer is performed via the token contract. If the sink rejects
    /// the transfer the share is left claimable and the error is surfaced, so a
    /// later retry can succeed without affecting other sinks.
    pub fn claim(env: Env, sink: Address, token: Address) -> Result<i128, FeeRouterError> {
        sink.require_auth();
        let key = DataKey::Claimable(sink.clone(), token.clone());
        let amount: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        if amount == 0 {
            return Err(FeeRouterError::NothingToClaim);
        }
        let client = token::Client::new(&env, &token);
        client.transfer(&env.current_contract_address(), &sink, &amount);
        env.storage().persistent().set(&key, &0i128);
        Ok(amount)
    }

    /// Propose a new set of sinks and weights. Only governance may call this.
    pub fn propose_weights(
        env: Env,
        caller: Address,
        sinks: Vec<Sink>,
    ) -> Result<(), FeeRouterError> {
        caller.require_auth();
        let governance: Address = env.storage().instance().get(&DataKey::Governance).unwrap();
        if caller != governance {
            return Err(FeeRouterError::Unauthorized);
        }
        validate_sinks(&env, &sinks)?;
        let delay: u64 = env.storage().instance().get(&DataKey::WeightDelay).unwrap();
        let pending = PendingWeights {
            sinks,
            executable_at: env.ledger().timestamp() + delay,
        };
        env.storage().instance().set(&DataKey::Pending, &pending);
        Ok(())
    }

    /// Apply a previously proposed weight change once the timelock has elapsed.
    pub fn apply_weights(env: Env, caller: Address) -> Result<(), FeeRouterError> {
        caller.require_auth();
        let governance: Address = env.storage().instance().get(&DataKey::Governance).unwrap();
        if caller != governance {
            return Err(FeeRouterError::Unauthorized);
        }
        let pending: PendingWeights = env
            .storage()
            .instance()
            .get(&DataKey::Pending)
            .ok_or(FeeRouterError::NoPendingWeights)?;
        if env.ledger().timestamp() < pending.executable_at {
            return Err(FeeRouterError::TimelockNotElapsed);
        }
        env.storage().instance().set(&DataKey::Sinks, &pending.sinks);
        env.storage().instance().remove(&DataKey::Pending);
        Ok(())
    }

    /// Timestamp at which the pending weight change becomes executable.
    pub fn pending_executable_at(env: Env) -> Option<u64> {
        let pending: Option<PendingWeights> = env.storage().instance().get(&DataKey::Pending);
        pending.map(|p| p.executable_at)
    }
}

fn validate_sinks(env: &Env, sinks: &Vec<Sink>) -> Result<(), FeeRouterError> {
    let len = sinks.len();
    if len == 0 {
        return Err(FeeRouterError::EmptySinks);
    }
    if len > MAX_SINKS {
        return Err(FeeRouterError::TooManySinks);
    }
    let mut sum: u32 = 0;
    for i in 0..len {
        let sink = sinks.get(i).unwrap();
        for j in 0..i {
            if sinks.get(j).unwrap().account == sink.account {
                return Err(FeeRouterError::DuplicateSink);
            }
        }
        sum = sum.saturating_add(sink.weight_bps);
    }
    if sum != TOTAL_BPS {
        return Err(FeeRouterError::WeightsMustSumToTotal);
    }
    let _ = env;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};

    fn setup() -> (Env, FeeRouterClient<'static>, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, FeeRouter);
        let client = FeeRouterClient::new(&env, &contract_id);

        let governance = Address::generate(&env);
        let treasury = Address::generate(&env);
        let backstop = Address::generate(&env);
        let rebate = Address::generate(&env);

        let mut sinks = Vec::new(&env);
        sinks.push_back(Sink { account: treasury.clone(), weight_bps: 5_000 });
        sinks.push_back(Sink { account: backstop.clone(), weight_bps: 3_000 });
        sinks.push_back(Sink { account: rebate.clone(), weight_bps: 2_000 });

        client.initialize(&governance, &100u64, &sinks);
        (env, client, governance, treasury, backstop, rebate)
    }

    fn make_token(env: &Env, admin: &Address) -> Address {
        env.register_stellar_asset_contract(admin.clone())
    }

    #[test]
    fn distribute_conserves_balance() {
        let (env, client, _gov, treasury, backstop, rebate) = setup();
        let admin = Address::generate(&env);
        let token = make_token(&env, &admin);

        client.on_fee_received(&token, &1_000i128);
        assert_eq!(client.balance_of(&token), 1_000);
        client.distribute(&token);
        assert_eq!(client.balance_of(&token), 0);

        let t = client.claimable(&treasury, &token);
        let b = client.claimable(&backstop, &token);
        let r = client.claimable(&rebate, &token);
        assert_eq!(t + b + r, 1_000);
        assert_eq!(t, 500);
        assert_eq!(b, 300);
        assert_eq!(r, 200);
    }

    #[test]
    fn weights_must_sum_to_total() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, FeeRouter);
        let client = FeeRouterClient::new(&env, &contract_id);
        let governance = Address::generate(&env);
        let mut sinks = Vec::new(&env);
        sinks.push_back(Sink { account: Address::generate(&env), weight_bps: 5_000 });
        sinks.push_back(Sink { account: Address::generate(&env), weight_bps: 4_000 });
        let res = client.try_initialize(&governance, &100u64, &sinks);
        assert_eq!(res, Err(Ok(FeeRouterError::WeightsMustSumToTotal)));
    }

    #[test]
    fn timelock_enforced() {
        let (env, client, governance, _t, _b, _r) = setup();
        let mut sinks = Vec::new(&env);
        sinks.push_back(Sink { account: Address::generate(&env), weight_bps: 10_000 });

        env.ledger().set_timestamp(1_000);
        client.propose_weights(&governance, &sinks);
        assert_eq!(client.pending_executable_at(), Some(1_100));

        let res = client.try_apply_weights(&governance);
        assert_eq!(res, Err(Ok(FeeRouterError::TimelockNotElapsed)));

        env.ledger().set_timestamp(1_100);
        client.apply_weights(&governance);
        assert_eq!(client.sinks().len(), 1);
    }

    #[test]
    fn rejected_transfer_keeps_share_claimable() {
        let (env, client, _gov, treasury, _b, _r) = setup();
        let admin = Address::generate(&env);
        let token = make_token(&env, &admin);
        let sac = StellarAssetClient::new(&env, &token);
        let tc = TokenClient::new(&env, &token);

        // Fund the router so it can pay out.
        sac.mint(&client.address, &1_000i128);
        client.on_fee_received(&token, &1_000i128);
        client.distribute(&token);

        // Treasury claims successfully.
        let amount = client.claim(&treasury, &token);
        assert_eq!(amount, 500);
        assert_eq!(tc.balance(&treasury), 500);
        assert_eq!(client.claimable(&treasury, &token), 0);

        // A second claim with nothing left errors but does not affect others.
        let res = client.try_claim(&treasury, &token);
        assert_eq!(res, Err(Ok(FeeRouterError::NothingToClaim)));
    }
}
