//! Integration tests for `backstop_vault::get_lp_position` — Issue #533.
//!
//! `get_lp_position(lp)` used to return `(share_balance, queued_withdrawal_amount)`.
//! The withdrawal-cooldown timestamp that `withdraw` itself enforces lived only
//! in the private storage key `DataKey::WithdrawCooldown(lp)`, so an LP (or a
//! dashboard) had no on-chain way to learn when their next `withdraw` would be
//! accepted.
//!
//! The view now returns `(share_balance, queued_withdrawal_amount,
//! withdraw_cooldown_until)`. These tests pin down:
//!
//! * an LP with no history reports zeroes,
//! * the reported timestamp is exactly `deposit_timestamp + WITHDRAWAL_COOLDOWN_SECS`,
//! * the reported timestamp is the *same* value `withdraw` compares against —
//!   proven by driving `withdraw` to the exact boundary of the reported value,
//! * a later deposit resets the window,
//! * the view is read-only, and
//! * the second tuple element (the emergency-mode withdrawal queue) is still
//!   reported correctly.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};
use vortex_backstop_vault::{BackstopVault, BackstopVaultClient, Error};

/// `backstop_vault::WITHDRAWAL_COOLDOWN_SECS` is a private constant, so the
/// test mirrors the value the contract uses (48 hours, in seconds).
const WITHDRAWAL_COOLDOWN_SECS: u64 = 172_800;

/// A deposit large enough to clear the contract's `MIN_DEPOSIT` (1 USDC at
/// 7 decimals).
const DEPOSIT: i128 = 100 * 10_000_000;

/// The virtual shares/assets the vault is seeded with at initialization
/// (`VIRTUAL_SHARES` / `VIRTUAL_ASSETS` are private constants).
const VIRTUAL: i128 = 1_000_000_000;

/// Ledger timestamp used as "now" for the deposits in these tests.
const T0: u64 = 1_000_000;

// ── Fixture ────────────────────────────────────────────────────────────────────

struct Ctx {
    env: Env,
    admin: Address,
    lp: Address,
    bond_token: Address,
    contract_id: Address,
}

impl Ctx {
    fn client(&self) -> BackstopVaultClient<'_> {
        BackstopVaultClient::new(&self.env, &self.contract_id)
    }

    /// Mint `amount` of the bond token to the LP so it can deposit.
    fn fund_lp(&self, amount: i128) {
        token::StellarAssetClient::new(&self.env, &self.bond_token).mint(&self.lp, &amount);
    }
}

fn setup() -> Ctx {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let lp = Address::generate(&env);
    let settlement = Address::generate(&env);
    let bond_token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_id = env.register_contract(None, BackstopVault);

    let ctx = Ctx {
        env,
        admin,
        lp,
        bond_token,
        contract_id,
    };

    ctx.client()
        .initialize(&ctx.admin, &ctx.bond_token, &settlement);
    ctx
}

// ── Tests ──────────────────────────────────────────────────────────────────────

/// An LP that has never interacted with the vault has no shares, nothing
/// queued and no cooldown — the view reports zeroes rather than panicking.
#[test]
fn get_lp_position_reports_zeroes_for_an_unknown_lp() {
    let ctx = setup();

    let (shares, queued, cooldown_until) = ctx.client().get_lp_position(&ctx.lp);

    assert_eq!(shares, 0);
    assert_eq!(queued, 0);
    assert_eq!(cooldown_until, 0);
}

/// After a deposit the view reports the shares minted and the exact moment
/// the withdrawal cooldown ends: `deposit_timestamp + WITHDRAWAL_COOLDOWN_SECS`.
#[test]
fn get_lp_position_reports_the_cooldown_after_a_deposit() {
    let ctx = setup();
    ctx.env.ledger().set_timestamp(T0);
    ctx.fund_lp(DEPOSIT);

    let minted = ctx.client().deposit(&ctx.lp, &DEPOSIT);
    assert!(minted > 0, "deposit should mint shares");

    let (shares, queued, cooldown_until) = ctx.client().get_lp_position(&ctx.lp);

    assert_eq!(shares, minted);
    assert_eq!(queued, 0);
    assert_eq!(cooldown_until, T0 + WITHDRAWAL_COOLDOWN_SECS);
}

/// The reported timestamp is the value `withdraw` actually enforces.
///
/// The test walks `withdraw` right up to the boundary of the reported
/// timestamp: one second before it the call must fail with
/// `Error::CooldownNotExpired`, and exactly at it the call must succeed. A
/// view that returned a different (stale, off-by-one, or unrelated) number
/// would fail this test.
#[test]
fn get_lp_position_reports_the_cooldown_that_withdraw_enforces() {
    let ctx = setup();
    ctx.env.ledger().set_timestamp(T0);
    ctx.fund_lp(DEPOSIT);

    ctx.client().deposit(&ctx.lp, &DEPOSIT);
    let (_, _, cooldown_until) = ctx.client().get_lp_position(&ctx.lp);
    assert_eq!(cooldown_until, T0 + WITHDRAWAL_COOLDOWN_SECS);

    // One second before the reported cooldown the withdrawal is refused.
    ctx.env.ledger().set_timestamp(cooldown_until - 1);
    assert_eq!(
        ctx.client().try_withdraw(&ctx.lp, &1),
        Err(Ok(Error::CooldownNotExpired.into())),
        "withdraw must be refused while now < the reported cooldown_until"
    );

    // Exactly at the reported cooldown the withdrawal is accepted.
    ctx.env.ledger().set_timestamp(cooldown_until);
    ctx.client().withdraw(&ctx.lp, &1);
}

/// A second deposit starts a *new* cooldown window: the reported timestamp
/// moves to the new deposit's timestamp plus the cooldown, it is not extended
/// from the first deposit's window.
#[test]
fn get_lp_position_cooldown_resets_on_a_second_deposit() {
    let ctx = setup();
    ctx.env.ledger().set_timestamp(T0);
    ctx.fund_lp(DEPOSIT * 2);

    ctx.client().deposit(&ctx.lp, &DEPOSIT);
    let (_, _, first_cooldown) = ctx.client().get_lp_position(&ctx.lp);
    assert_eq!(first_cooldown, T0 + WITHDRAWAL_COOLDOWN_SECS);

    let t1 = T0 + 1_000;
    ctx.env.ledger().set_timestamp(t1);
    ctx.client().deposit(&ctx.lp, &DEPOSIT);

    let (shares, _, second_cooldown) = ctx.client().get_lp_position(&ctx.lp);
    assert_eq!(
        second_cooldown,
        t1 + WITHDRAWAL_COOLDOWN_SECS,
        "a new deposit starts a fresh cooldown window"
    );
    assert!(
        second_cooldown != first_cooldown,
        "the cooldown must be re-stamped, not left at the old value"
    );
    assert!(shares > 0);

    // The freshly reported cooldown is again the one that is enforced.
    ctx.env.ledger().set_timestamp(second_cooldown - 1);
    assert_eq!(
        ctx.client().try_withdraw(&ctx.lp, &1),
        Err(Ok(Error::CooldownNotExpired.into()))
    );
}

/// The view is read-only: calling it never mutates the vault's state, and in
/// particular it does not create, extend or clear the cooldown entry, nor
/// does it make an ineligible LP eligible.
#[test]
fn get_lp_position_is_read_only() {
    let ctx = setup();
    ctx.env.ledger().set_timestamp(T0);
    ctx.fund_lp(DEPOSIT);

    ctx.client().deposit(&ctx.lp, &DEPOSIT);
    let before = ctx.client().get_lp_position(&ctx.lp);

    for _ in 0..5 {
        assert_eq!(ctx.client().get_lp_position(&ctx.lp), before);
    }

    // The vault's aggregate state is untouched by the view. The totals include
    // the virtual shares/assets the vault is seeded with at initialization
    // (`VIRTUAL_SHARES` / `VIRTUAL_ASSETS`, both private constants worth
    // 1_000_000_000).
    let (total_assets, total_shares, _) = ctx.client().get_vault_state();
    assert_eq!(total_assets, DEPOSIT + VIRTUAL);
    assert_eq!(total_shares, before.0 + VIRTUAL);

    // Reading the position did not waive the cooldown.
    ctx.env.ledger().set_timestamp(T0 + 1);
    assert_eq!(
        ctx.client().try_withdraw(&ctx.lp, &1),
        Err(Ok(Error::CooldownNotExpired.into()))
    );
}

/// The second tuple element still reports the emergency-mode withdrawal queue
/// correctly, alongside the new cooldown element.
#[test]
fn get_lp_position_reports_the_emergency_withdrawal_queue() {
    let ctx = setup();
    ctx.env.ledger().set_timestamp(T0);
    ctx.fund_lp(DEPOSIT);

    let minted = ctx.client().deposit(&ctx.lp, &DEPOSIT);

    // Emergency mode queues withdrawals instead of executing them.
    ctx.client().set_emergency_mode(&true);

    let (_, _, cooldown_until) = ctx.client().get_lp_position(&ctx.lp);
    ctx.env.ledger().set_timestamp(cooldown_until);
    let queued = ctx.client().withdraw(&ctx.lp, &minted);
    assert!(queued > 0);

    let (shares, queued_amount, cooldown_until) = ctx.client().get_lp_position(&ctx.lp);

    assert_eq!(
        shares, 0,
        "queued withdrawals burn the LP's shares immediately"
    );
    assert_eq!(queued_amount, queued);
    assert_eq!(cooldown_until, T0 + WITHDRAWAL_COOLDOWN_SECS);
}
