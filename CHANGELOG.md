# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- **`governor` contract — on-chain proposal/voting lifecycle (issue #508).**
  New `governor/` crate implementing proposal creation, per-address voting,
  quorum tracking, and permissionless execution once a proposal passes. Core
  state machine (`Pending → Active → Succeeded/Defeated → Queued → Executed`)
  is present. Known gaps: quorum/threshold constants are not yet tuneable via
  admin config, and the execution hook into `intent_settlement` admin operations
  is not wired. Not yet production-ready; tracked in issue #508.

- **`backstop_vault` contract — last-resort collateral reserve (issue #508).**
  New `backstop_vault/` crate providing a deposit/withdraw vault that can be
  drawn on to cover under-collateralised slash shortfalls. Collusion-risk
  mitigations documented in `docs/369-backstop-collusion-analysis.md` are not
  yet fully implemented; the redemption-under-shortfall path requires further
  hardening before mainnet use. Not yet production-ready; tracked in issue #508.

- **`treasury_streams` contract — continuous protocol-fee streaming (issue #508).**
  New `treasury_streams/` crate supporting creation and cancellation of
  per-second token streams to designated recipients. Stream creation and
  cancellation are functional. Rate-change governance hook and safe-transfer
  edge cases (zero-balance flush, stream expiry without explicit cancel) need
  review. Not yet production-ready; tracked in issue #508.

- **`fee_router` contract — protocol-fee distribution router (issue #508).**
  New `fee_router/` crate routing incoming protocol fees to `treasury_streams`
  and/or `backstop_vault` according to an admin-configured split ratio. Core
  routing logic is present. Split-update timelock and multi-recipient input
  validation are incomplete. Not yet production-ready; tracked in issue #508.

- **`timelock` contract — standalone timelock controller (issue #508).**
  New `timelock/` crate providing a configurable-delay execution controller,
  distinct from the `propose_*/execute_*` pattern embedded in
  `intent_settlement`. Intended to wrap `governor` proposal execution. Delay
  parameterisation and the cancellation path require final review before
  production use. Not yet production-ready; tracked in issue #508.

- **`intent_periphery` contract — intent convenience layer (issue #508).**
  New `intent_periphery/` crate exposing higher-level helpers built on top of
  `intent_settlement` (e.g. single-call submit-and-accept flows for testing and
  integrations). Not yet production-ready; tracked in issue #508.

- **CI: Enforce CHANGELOG.md updates (issue #294).** A new CI job verifies that
  every PR changing `intent_settlement/src/` or `proof_registry/src/` includes a
  corresponding update to `CHANGELOG.md`. An escape hatch (`no-changelog-needed`
  label) exists for genuinely changelog-exempt changes (pure test-only, 
  comment-only, CI/tooling).
- **CI: Soroban standalone network integration test (issue #295).** A new CI job
  runs a full end-to-end lifecycle test (`register_solver` → `submit_intent` →
  `accept_intent` → `fill_intent`) against a local Soroban standalone network on
  every PR. Exercises the real deployment, contract initialization, and CLI-invocation
  path — catching issues that in-process unit tests miss. Includes a local test
  script (`scripts/e2e-test.sh`) for debugging; documented in `CONTRIBUTING.md`.
- **`.github/CODEOWNERS` file establishing review routing (issue #296).** Maps
  major repository areas (`intent_settlement/`, `proof_registry/`, `solver_registry/`,
  `indexer/`, `docs/`, `.github/workflows/`, etc.) to code owners/teams. PRs
  automatically request review from owners with relevant expertise. Documented
  process for contributors to become owners in `CONTRIBUTING.md`.
- **Formalized off-chain governance process (issue #297).** New `GOVERNANCE.md`
  documents the RFC and discussion process preceding on-chain proposals. Establishes
  minimum 3-business-day discussion window before `propose_*` calls; describes what
  requires a proposal (fee changes, admin transfers, upgrades, token allowlist changes),
  emergency exception for `pause()`, and rollback planning. Includes GitHub issue
  template for proposals (`.github/ISSUE_TEMPLATE/governance-proposal.md`). Referenced
  from `CONTRIBUTING.md` and `README.md`.

### Changed

- Upgraded `soroban-sdk` from `21.0.0` to `22.0.0` across the workspace root and every contract crate (`intent_settlement`, `proof_registry`, `solver_registry`, `reputation_badge`, `backstop_vault`), closing the gap left by issue #377. Migration checklist followed from `docs/event-schema.md` (typed events, test registration API, TTL/auth API notes).
- **`fee_router` rewritten as a real Soroban contract (issue #464).**
  `fee_router/src/lib.rs` is now a deployable `#![no_std]` contract with
  `#[contract]`/`#[contractimpl]` instead of plain `std` Rust. Sinks and the
  governance address are `soroban_sdk::Address` (was `String`), transfers go
  through `token::Client` (was a caller-supplied `FnOnce` closure), and all
  state (weights, governance, timelock, claimable balances) lives in Soroban
  storage (was in-memory `BTreeMap`s). The pull-based `claimable` accounting,
  timelocked `propose_weights`/`apply_weights`, and dust-to-first-sink
  conservation are preserved. The inline test scenarios
  (`distribute_conserves_balance`, `weights_must_sum_to_total`,
  `timelock_enforced`, `rejected_transfer_keeps_share_claimable`) are ported to
  `soroban_sdk::testutils`. Fee-on-transfer tokens are handled by crediting the
  amount actually received (balance delta) rather than the requested amount.
- **Storage layout — `SolverRecord` / `IntentRecord` (issue #187, #188).**
  `SolverRecord` gains a `bond_tokens: Vec<Address>` field enumerating every
  approved token a solver holds a bond in; `bond_amount` is retained as the
  mirror of the *default* token's balance for backward compatibility, with
  non-default balances living under the new `DataKey::SolverBond(solver, token)`
  persistent key. `IntentRecord` gains `bond_token`, `dispute_deadline`,
  `dispute_raised_at`, and `resolution`. New `DataKey` variants:
  `BidWindowEnabled`, `BestBid`, `Arbiter`, `AllowedBondToken`, `SolverBond`,
  `MinBond`. Contracts upgrading from a pre-#187 build read old `SolverRecord`
  values with `bond_tokens` defaulting to empty and the legacy `bond_amount`
  intact; the first `register_solver*` call re-materialises `bond_tokens`.
- **`slash_solver` is now proportional (issue #193).** The flat 10 %-of-bond
  slash is replaced by `min(unfilled_output, bond) / 10`, capped at 10 % of the
  bond and floored at 1 stroop (issue #32). The `solver_slashed` event payload
  is unchanged.
- **`is_bid_window_enabled` no longer reads `DataKey::DstAllowlistEnabled`
  (issue #191).** It now reads a dedicated `DataKey::BidWindowEnabled`, closing
  a storage-key collision where `set_dst_allowlist_enabled` also toggled
  bid-window mode. `set_dst_allowlist_enabled` and `set_bid_window_enabled` are
  fully independent.
- **`fill_intent` de-duplicated.** A bad merge had left the output/fee transfer
  block written three times; it now transfers once, after all state is
  committed (checks-effects-interactions).

### Added

- **Competitive bid window (issue #191).** `bid_intent(solver, intent_id,
  quoted_dst_amount)` records the strictly-highest quote for an intent in
  `Bidding` state (ties keep the incumbent); `settle_bids(intent_id)` is
  permissionless and either promotes the winner to `Accepted` with a fresh fill
  window or re-opens the intent as `Open` when no usable bid exists. Toggle via
  `set_bid_window_enabled`; view via `is_bid_window_enabled` and `get_best_bid`.
- **Dispute-resolution flow (issue #188, docs/dispute-resolution-design.md).**
  New states `Filling`, `Disputed`, `Resolved` and enum `DisputeResolution`.
  `begin_fill` escrows a completing fill in the contract and opens a
  `DISPUTE_WINDOW`; `dispute_fill` lets the user contest it; `resolve_dispute`
  (arbiter-only — `set_arbiter` / `get_arbiter`, defaults to admin) rules
  `Upheld` (proportional slash) or `Dismissed` (fee taken, no slash);
  `release_fill` is the permissionless clean-release / arbiter-timeout path. The
  user receives the escrowed tokens in every outcome.
- **Multi-bond-token support (issue #187, docs/60-multi-bond-token-design.md).**
  `add_allowed_bond_token` / `remove_allowed_bond_token` /
  `set_bond_token_min` / `get_bond_token_min`, plus token-aware
  `register_solver_with_token`, `withdraw_bond_token`, `accept_intent_with_bond`
  and the `get_solver_bond` / `get_solver_bonds` views. Bonds, minimums, and
  slashes are all accounted per token; the slash for an intent is taken from —
  and paid out in — the token the solver bonded when accepting it. Solvers may
  hold bonds in up to `MAX_BOND_TOKENS` (8) distinct tokens;
  `deregister_solver` refunds every one.

### Fixed

- **Compiling, green baseline (#202, #203, #204)**: `intent_settlement` did
  not build — `lib.rs` referenced ~12 undeclared constants, 9 undeclared
  `DataKey` variants and 6 undeclared `Error` variants, the `Error` enum had
  duplicate discriminants, `validate_src_token` called a non-existent
  `String::get`, `compute_reputation_score` was a `pub` contract fn taking a
  non-ABI `&SolverRecord`, and `fill_intent` transferred the fill amount and
  fee three times each. Constants are now declared with rationale comments,
  the enums are reconciled (discriminants renumbered sequentially), the
  string validation reads bytes via `copy_into_slice`, and `fill_intent`
  makes exactly one user transfer and one fee transfer. The test suite and
  the bond-conservation proptest, both damaged by earlier bad merges, are
  repaired.
- **Batch operations were unusable for more than one item**:
  `batch_submit_intent` / `batch_accept_intent` called `require_auth()` once
  per loop iteration, which Soroban rejects with `Auth, ExistingValue` on the
  second item. The batch entrypoints now authorise the actor once and invoke
  un-gated `*_inner` bodies.
- `deregister_solver` now refuses to return a solver's bond while they hold
  an `Accepted` intent, closing a path to dodge `slash_solver` by
  withdrawing before the fill window expired.
- `register_solver` checks the *cumulative* bond total against `MIN_BOND`
  instead of each individual deposit, so a solver already above the
  minimum can top up by a smaller amount without being wrongly rejected.
- A solver whose bond falls below `MIN_BOND` after a slash is now
  automatically deactivated, rather than staying eligible to accept
  further intents while under-collateralized.

### Added

- **#240** (`proof_registry`): Contract upgrade mechanism (`upgrade` entrypoint
  and `migrate()` guard) matching the pattern in `intent_settlement`. Allows 
  `proof_registry` to evolve without data loss or re-initialization. Migration 
  guard prevents double-execution on the same version.
- **Storage TTL management**: persistent `Intent`/`Solver` entries and the 
  contract instance now have their TTL extended on every write, closing a 
  gap where none of Soroban's state-archival requirements were handled.
- **Admin key management**: `set_fee_recipient`, `transfer_admin`
  (requires auth from both the outgoing and incoming admin), and 
  `get_admin`/`get_fee_recipient` views -- previously no rotation path 
  existed for either role.
- **Emergency pause**: `pause()`/`unpause()`/`is_paused()`,
