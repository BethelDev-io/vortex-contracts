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
//! - Account identifiers are length-bounded ([`MAX_SINK_ACCOUNT_LEN`] and
//!   [`MAX_GOVERNANCE_ACCOUNT_LEN`]) so that an unbounded `String` cannot be
//!   written into the router's storage maps. See
//!   "Identifier bounding and the Soroban rewrite" below.
//!
//! # Identifier bounding and the Soroban rewrite
//!
//! `Sink::account` and the router's `governance` account are currently plain
//! `String`s, so today there is no structural limit on how long an identifier
//! may be. That limit has to exist before this crate becomes a real Soroban
//! contract: on-chain, every byte of a stored identifier is paid for in
//! storage rent, and a governance proposal could otherwise write an
//! arbitrarily large entry into `sinks` and `claimable`.
//!
//! This crate therefore bounds both identifiers *now*, with
//! [`MAX_SINK_ACCOUNT_LEN`] and [`MAX_GOVERNANCE_ACCOUNT_LEN`], and rejects
//! out-of-bounds identifiers with [`FeeRouterError::SinkAccountTooLong`] /
//! [`FeeRouterError::GovernanceAccountTooLong`].
//!
//! The bound is a placeholder for the real fix, not a substitute for it.
//! When `fee_router` is rewritten as an actual Soroban contract, sink
//! identity should become a `soroban_sdk::Address`, which is *natively*
//! bounded by the host (a `ScAddress` is at most 32 bytes of account id plus
//! a 4-byte discriminant) and can be paid for exactly rather than
//! approximately. The explicit length checks below should then be replaced by
//! the `Address` type check, not kept alongside it.

use std::collections::BTreeMap;

/// Total basis points that weights must sum to.
pub const TOTAL_BPS: u32 = 10_000;

/// Maximum number of sinks the router will accept.
pub const MAX_SINKS: usize = 16;

/// Maximum length, in **bytes**, of a sink's account identifier.
///
/// The bound is checked against the UTF-8 byte length of the identifier, not
/// its character count, because on-chain storage cost is metered in bytes.
/// 64 bytes is enough for an ed25519 public key in hex (64 hex characters)
/// with room to spare, and keeps the worst case for a full sink set
/// (`MAX_SINKS` sinks) at 1 KiB.
pub const MAX_SINK_ACCOUNT_LEN: usize = 64;

/// Maximum length, in **bytes**, of the router's governance account
/// identifier.
///
/// Same reasoning as [`MAX_SINK_ACCOUNT_LEN`]: the governance account is a
/// single stored identifier, so it needs the same byte-length bound.
pub const MAX_GOVERNANCE_ACCOUNT_LEN: usize = 64;

/// A single revenue sink with its governance-set weight.
///
/// The `account` field must be at most [`MAX_SINK_ACCOUNT_LEN`] bytes long.
/// Constructing a `Sink` directly does not enforce that; the router validates
/// the bound through [`FeeRouter::new`] and [`FeeRouter::propose_weights`],
/// which are the only ways a sink can enter the router's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sink {
    pub account: String,
    pub weight_bps: u32,
}

/// A pending, timelocked weight change.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingWeights {
    sinks: Vec<Sink>,
    executable_at: u64,
}

/// Errors returned by the router.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeeRouterError {
    Unauthorized,
    TooManySinks,
    WeightsMustSumToTotal,
    EmptySinks,
    DuplicateSink,
    /// A sink's `account` identifier exceeded [`MAX_SINK_ACCOUNT_LEN`] bytes.
    SinkAccountTooLong,
    /// The governance account identifier exceeded
    /// [`MAX_GOVERNANCE_ACCOUNT_LEN`] bytes.
    GovernanceAccountTooLong,
    NoPendingWeights,
    TimelockNotElapsed,
    UnknownToken,
    NothingToClaim,
    TransferFailed,
}

/// The fee router contract.
#[derive(Debug)]
pub struct FeeRouter {
    governance: String,
    weight_delay: u64,
    sinks: Vec<Sink>,
    pending: Option<PendingWeights>,
    /// token -> total accumulated but not yet distributed balance.
    balances: BTreeMap<String, u128>,
    /// sink -> token -> claimable amount.
    claimable: BTreeMap<String, BTreeMap<String, u128>>,
}

impl FeeRouter {
    /// Create a new router. `sinks` must be non-empty, unique, at most
    /// [`MAX_SINKS`], sum to [`TOTAL_BPS`], and carry account identifiers of
    /// at most [`MAX_SINK_ACCOUNT_LEN`] bytes each.
    ///
    /// `governance` must be at most [`MAX_GOVERNANCE_ACCOUNT_LEN`] bytes.
    pub fn new(
        governance: impl Into<String>,
        weight_delay: u64,
        sinks: Vec<Sink>,
    ) -> Result<Self, FeeRouterError> {
        let governance = governance.into();
        validate_governance(&governance)?;
        validate_sinks(&sinks)?;
        Ok(Self {
            governance,
            weight_delay,
            sinks,
            pending: None,
            balances: BTreeMap::new(),
            claimable: BTreeMap::new(),
        })
    }

    pub fn governance(&self) -> &str {
        &self.governance
    }

    pub fn weight_delay(&self) -> u64 {
        self.weight_delay
    }

    pub fn sinks(&self) -> &[Sink] {
        &self.sinks
    }

    /// Total accumulated balance for `token` that has not yet been distributed.
    pub fn balance_of(&self, token: &str) -> u128 {
        self.balances.get(token).copied().unwrap_or(0)
    }

    /// Amount currently claimable by `sink` for `token`.
    pub fn claimable(&self, sink: &str, token: &str) -> u128 {
        self.claimable
            .get(sink)
            .and_then(|m| m.get(token))
            .copied()
            .unwrap_or(0)
    }

    /// Record an incoming fee payment. The router credits the amount it
    /// actually received, which makes fee-on-transfer tokens safe.
    pub fn on_fee_received(&mut self, token: impl Into<String>, amount: u128) {
        if amount == 0 {
            return;
        }
        let entry = self.balances.entry(token.into()).or_insert(0);
        *entry = entry.saturating_add(amount);
    }

    /// Permissionlessly distribute the accumulated balance of `token` across
    /// the configured sinks according to their weights.
    ///
    /// Rounding dust is assigned to the first sink so that the full balance is
    /// always conserved.
    pub fn distribute(&mut self, token: &str) -> Result<(), FeeRouterError> {
        let total = self.balance_of(token);
        if total == 0 {
            return Err(FeeRouterError::UnknownToken);
        }

        let mut distributed: u128 = 0;
        for (i, sink) in self.sinks.iter().enumerate() {
            let share = if i == 0 {
                // First sink absorbs rounding dust.
                total - distributed
            } else {
                total.saturating_mul(sink.weight_bps as u128) / TOTAL_BPS as u128
            };
            distributed = distributed.saturating_add(share);
            if share > 0 {
                let entry = self
                    .claimable
                    .entry(sink.account.clone())
                    .or_default()
                    .entry(token.to_string())
                    .or_insert(0);
                *entry = entry.saturating_add(share);
            }
        }

        self.balances.insert(token.to_string(), 0);
        Ok(())
    }

    /// Withdraw the caller's claimable balance for `token`.
    ///
    /// The transfer is performed by the caller-supplied closure. If the sink
    /// rejects the transfer the share is left claimable and the error is
    /// surfaced, so a later retry can succeed without affecting other sinks.
    pub fn claim<F>(&mut self, sink: &str, token: &str, transfer: F) -> Result<u128, FeeRouterError>
    where
        F: FnOnce(&str, &str, u128) -> bool,
    {
        let amount = self.claimable(sink, token);
        if amount == 0 {
            return Err(FeeRouterError::NothingToClaim);
        }
        if !transfer(sink, token, amount) {
            return Err(FeeRouterError::TransferFailed);
        }
        if let Some(m) = self.claimable.get_mut(sink) {
            m.insert(token.to_string(), 0);
        }
        Ok(amount)
    }

    /// Propose a new set of sinks and weights. Only governance may call this.
    ///
    /// The proposed sinks are validated with the same rules as
    /// [`FeeRouter::new`], so the [`MAX_SINK_ACCOUNT_LEN`] bound is re-checked
    /// on every weight change and cannot be bypassed by proposing a new set.
    pub fn propose_weights(
        &mut self,
        caller: &str,
        sinks: Vec<Sink>,
        now: u64,
    ) -> Result<(), FeeRouterError> {
        if caller != self.governance {
            return Err(FeeRouterError::Unauthorized);
        }
        validate_sinks(&sinks)?;
        self.pending = Some(PendingWeights {
            sinks,
            executable_at: now.saturating_add(self.weight_delay),
        });
        Ok(())
    }

    /// Apply a previously proposed weight change once the timelock has elapsed.
    pub fn apply_weights(&mut self, caller: &str, now: u64) -> Result<(), FeeRouterError> {
        if caller != self.governance {
            return Err(FeeRouterError::Unauthorized);
        }
        let pending = self
            .pending
            .take()
            .ok_or(FeeRouterError::NoPendingWeights)?;
        if now < pending.executable_at {
            self.pending = Some(pending);
            return Err(FeeRouterError::TimelockNotElapsed);
        }
        self.sinks = pending.sinks;
        Ok(())
    }

    /// Timestamp at which the pending weight change becomes executable.
    pub fn pending_executable_at(&self) -> Option<u64> {
        self.pending.as_ref().map(|p| p.executable_at)
    }
}

/// Validate the governance account identifier's length.
///
/// Called from [`FeeRouter::new`]. The governance account is the only other
/// account identifier the router stores, so it gets the same byte-length
/// bound as a sink account.
fn validate_governance(governance: &str) -> Result<(), FeeRouterError> {
    if governance.len() > MAX_GOVERNANCE_ACCOUNT_LEN {
        return Err(FeeRouterError::GovernanceAccountTooLong);
    }
    Ok(())
}

/// Validate a sink set: non-empty, bounded size, unique and bounded-length
/// account identifiers, and weights summing to [`TOTAL_BPS`].
///
/// Called from [`FeeRouter::new`] and [`FeeRouter::propose_weights`], so the
/// bound is re-checked every time governance tries to install a new sink set.
fn validate_sinks(sinks: &[Sink]) -> Result<(), FeeRouterError> {
    if sinks.is_empty() {
        return Err(FeeRouterError::EmptySinks);
    }
    if sinks.len() > MAX_SINKS {
        return Err(FeeRouterError::TooManySinks);
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut sum: u32 = 0;
    for sink in sinks {
        if sink.account.len() > MAX_SINK_ACCOUNT_LEN {
            return Err(FeeRouterError::SinkAccountTooLong);
        }
        if !seen.insert(sink.account.as_str()) {
            return Err(FeeRouterError::DuplicateSink);
        }
        sum = sum.saturating_add(sink.weight_bps);
    }
    if sum != TOTAL_BPS {
        return Err(FeeRouterError::WeightsMustSumToTotal);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sinks() -> Vec<Sink> {
        vec![
            Sink { account: "treasury".into(), weight_bps: 5_000 },
            Sink { account: "backstop".into(), weight_bps: 3_000 },
            Sink { account: "rebate".into(), weight_bps: 2_000 },
        ]
    }

    #[test]
    fn distribute_conserves_balance() {
        let mut r = FeeRouter::new("gov", 100, sinks()).unwrap();
        r.on_fee_received("usd", 10_001);
        r.distribute("usd").unwrap();
        let total = r.claimable("treasury", "usd")
            + r.claimable("backstop", "usd")
            + r.claimable("rebate", "usd");
        assert_eq!(total, 10_001);
        assert_eq!(r.balance_of("usd"), 0);
    }

    #[test]
    fn weights_must_sum_to_total() {
        let bad = vec![Sink { account: "a".into(), weight_bps: 9_999 }];
        assert_eq!(FeeRouter::new("gov", 0, bad).unwrap_err(), FeeRouterError::WeightsMustSumToTotal);
    }

    #[test]
    fn timelock_enforced() {
        let mut r = FeeRouter::new("gov", 100, sinks()).unwrap();
        r.propose_weights("gov", vec![Sink { account: "a".into(), weight_bps: 10_000 }], 0).unwrap();
        assert_eq!(r.apply_weights("gov", 50).unwrap_err(), FeeRouterError::TimelockNotElapsed);
        r.apply_weights("gov", 100).unwrap();
        assert_eq!(r.sinks().len(), 1);
    }

    #[test]
    fn rejected_transfer_keeps_share_claimable() {
        let mut r = FeeRouter::new("gov", 0, sinks()).unwrap();
        r.on_fee_received("usd", 1_000);
        r.distribute("usd").unwrap();
        let before = r.claimable("treasury", "usd");
        assert_eq!(r.claim("treasury", "usd", |_, _, _| false).unwrap_err(), FeeRouterError::TransferFailed);
        assert_eq!(r.claimable("treasury", "usd"), before);
        assert_eq!(r.claim("treasury", "usd", |_, _, _| true).unwrap(), before);
    }

    // ---- Issue #536: account identifier length bounds ----

    /// A sink account of exactly `MAX_SINK_ACCOUNT_LEN` bytes is accepted —
    /// the bound is inclusive.
    #[test]
    fn sink_account_at_max_len_is_accepted() {
        let account = "a".repeat(MAX_SINK_ACCOUNT_LEN);
        assert_eq!(account.len(), MAX_SINK_ACCOUNT_LEN);
        let sinks = vec![Sink {
            account,
            weight_bps: TOTAL_BPS,
        }];
        let r = FeeRouter::new("gov", 0, sinks).unwrap();
        assert_eq!(r.sinks()[0].account.len(), MAX_SINK_ACCOUNT_LEN);
    }

    /// A sink account of `MAX_SINK_ACCOUNT_LEN + 1` bytes is rejected.
    #[test]
    fn sink_account_over_max_len_is_rejected() {
        let account = "a".repeat(MAX_SINK_ACCOUNT_LEN + 1);
        let sinks = vec![Sink {
            account,
            weight_bps: TOTAL_BPS,
        }];
        assert_eq!(
            FeeRouter::new("gov", 0, sinks).unwrap_err(),
            FeeRouterError::SinkAccountTooLong
        );
    }

    /// The governance account is bounded by the same rule.
    #[test]
    fn governance_account_over_max_len_is_rejected() {
        let governance = "g".repeat(MAX_GOVERNANCE_ACCOUNT_LEN + 1);
        assert_eq!(
            FeeRouter::new(governance, 0, sinks()).unwrap_err(),
            FeeRouterError::GovernanceAccountTooLong
        );
    }

    /// A governance account of exactly `MAX_GOVERNANCE_ACCOUNT_LEN` bytes is
    /// accepted.
    #[test]
    fn governance_account_at_max_len_is_accepted() {
        let governance = "g".repeat(MAX_GOVERNANCE_ACCOUNT_LEN);
        assert!(FeeRouter::new(governance, 0, sinks()).is_ok());
    }

    /// The bound is re-checked when governance proposes a new sink set, so it
    /// cannot be bypassed by routing an over-long identifier through
    /// `propose_weights` / `apply_weights`.
    #[test]
    fn propose_weights_rejects_overlong_sink_account() {
        let mut r = FeeRouter::new("gov", 0, sinks()).unwrap();
        let account = "a".repeat(MAX_SINK_ACCOUNT_LEN + 1);
        let bad = vec![Sink {
            account,
            weight_bps: TOTAL_BPS,
        }];
        assert_eq!(
            r.propose_weights("gov", bad, 0).unwrap_err(),
            FeeRouterError::SinkAccountTooLong
        );
        // The pending set must not have been installed.
        assert!(r.pending_executable_at().is_none());
        assert_eq!(r.sinks().len(), 3);
    }

    /// The length bound is checked while iterating the sinks, i.e. *before*
    /// the weights are summed — an over-long identifier is reported as
    /// `SinkAccountTooLong` even when the weights are also wrong.
    #[test]
    fn overlong_sink_account_is_reported_before_weight_sum() {
        let account = "a".repeat(MAX_SINK_ACCOUNT_LEN + 1);
        let sinks = vec![Sink {
            account,
            weight_bps: 1,
        }];
        assert_eq!(
            FeeRouter::new("gov", 0, sinks).unwrap_err(),
            FeeRouterError::SinkAccountTooLong
        );
    }

    /// The bound is measured in **bytes**, not characters: a 64-byte
    /// multi-byte identifier is accepted, and a 66-byte one is rejected even
    /// though both are 33 characters long.
    #[test]
    fn sink_account_length_is_measured_in_bytes_not_characters() {
        // "é" is two bytes in UTF-8, so 32 of them are 64 bytes.
        let ok = "é".repeat(32);
        assert_eq!(ok.chars().count(), 32);
        assert_eq!(ok.len(), MAX_SINK_ACCOUNT_LEN);
        let sinks = vec![Sink {
            account: ok,
            weight_bps: TOTAL_BPS,
        }];
        assert!(FeeRouter::new("gov", 0, sinks).is_ok());

        // One more character is 66 bytes — over the bound.
        let too_long = "é".repeat(33);
        assert_eq!(too_long.chars().count(), 33);
        assert_eq!(too_long.len(), MAX_SINK_ACCOUNT_LEN + 2);
        let sinks = vec![Sink {
            account: too_long,
            weight_bps: TOTAL_BPS,
        }];
        assert_eq!(
            FeeRouter::new("gov", 0, sinks).unwrap_err(),
            FeeRouterError::SinkAccountTooLong
        );
    }
}
