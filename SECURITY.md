# Bug bounty

A draft program — in-scope assets, severity tiers, and (unfunded) reward
bands — is published in
[`docs/bug-bounty-program.md`](docs/bug-bounty-program.md). It is scoped
to the Assets at Risk table, Trust Assumptions, and the admin-key
blast-radius table in this file.

---

## Code of Conduct — financial-stakes supplement

This repository has a repo-specific [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
that supplements the org-wide `CONTRIBUTING.md` code-of-conduct reference. It
does not replace or supersede the org-wide policy; it adds conduct expectations
specific to a live, funds-bearing protocol. The sections below are the
security-relevant parts of that supplement, grounded in this file's threat
model.

### Responsible-disclosure conduct

The reporting instructions above ("do not open a public GitHub issue for
security vulnerabilities") are a conduct norm, not merely a suggestion. A
contributor who discloses an unpatched vulnerability in a public issue, PR,
chat, or social channel — including a partial hint sufficient to reproduce it —
has violated this code of conduct. Consequences are described in
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md#responsible-disclosure-conduct) and
range from a warning to removal from the contributor pool, and, where funds are
at risk, referral to the maintainers' incident process. Legitimate security
research is welcome and protected; exploitation of a live protocol is not.

### Privileged information and conflicts of interest

Anyone with admin, arbiter, or maintainer authority — and any reviewer who is
also a registered, actively-bonded solver — holds privileged knowledge that can
move real money. In particular, knowledge of a pending `propose_*` change
during its timelock window, of an unpatched vulnerability, or of an upcoming
admin action (pause, fee-recipient change, allowlist change) must not be used
for personal advantage, front-running, or tipping off a solver. The
conflict-of-interest and privileged-information standard, including the
arbiter-specific policy tracked in issue `#115`
([`docs/arbiter-code-of-conduct.md`](docs/arbiter-code-of-conduct.md)), lives in
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md#conflicts-of-interest-and-privileged-information).

### Enforcement and reporting

Conduct concerns — including suspected misuse of privileged information or a
public disclosure of an unpatched vulnerability — are reported through the
private channel described in [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md#enforcement-and-reporting),
not in a public issue thread. Security vulnerabilities themselves continue to
be reported per the instructions above.

---

## Threat Model — `intent_settlement`

This document describes the trust assumptions, assets at risk, and known
limitations of the `intent_settlement` contract ahead of a mainnet deployment.
It is intended for auditors and integrators.

---

### Assets at Risk

| Asset | Where held | Value |
|-------|-----------|-------|
| Solver bonds | `intent_settlement` contract account (USDC) | ≥ 50 USDC per solver |
| User swap output | Solver's token balance (transferred in `fill_intent`) | Unbounded |
| Protocol fees | `FeeRecipient` address | 0.05 % of filled volume |
| Admin privileges | `Admin` key in instance storage | Full contract control |

---

### Trust Assumptions

#### 1. Solver self-reporting

By default the contract has **no on-chain proof** that a solver actually sent
tokens on the source chain. `fill_intent` trusts the solver to transfer
`dst_token` to the user on Stellar; the economic incentive not to default is the
bond slash (`slash_solver` takes 10 % of the bond permissionlessly once the fill
window expires).

**Implication:** A solver can still accept an intent, fail to fill, absorb the
slash, and profit if the source-chain spread outweighs it. The proportional
formula removes the old asymmetry where a large bond made a small-intent default
disproportionately expensive (or a small bond made a large-intent default
cheap); the bond size remains the primary economic deterrent.

**Optional cryptographic gate (issue #190).** An admin can call
`set_proof_registry` to point `intent_settlement` at a deployed `ProofRegistry`
(Wormhole-backed). A solver may then call
`fill_intent(solver, intent_id, fill_amount, require_proof = true)`, which
cross-checks a verified source-chain deposit record before any token moves:

- no proof for the intent → `ProofNotFound`;
- proof's Wormhole `src_chain_id` ≠ mapped `intent.src_chain` → `ProofChainMismatch`;
- `proof.src_amount < intent.src_amount` → `ProofAmountInsufficient`;
- registry not configured → `ProofRegistryNotSet`.

Every failure path reverts before touching state, so the intent stays
`Accepted` and `slash_solver` remains the backstop (see
`docs/129-proof-mismatch-fallback.md`). This is **opt-in and defaults off**:
`require_proof = false` — the value every existing caller passes — reads no
registry and preserves the self-reported-fill behaviour above byte-for-byte,
mirroring how `DstAllowlistEnabled` defaults off. When the gate is used, trust
for that fill becomes *cryptographic (Guardian quorum) + economic (bond)* rather
than economic alone; residual risk is Guardian collusion, shared with all
Wormhole users.

#### 2. Admin key custody

A single `Admin` address controls:

- `pause` / `unpause` — can halt all new swaps.
- `set_fee_recipient` — redirects fee and slash proceeds.
- `transfer_admin` — rotates the admin key (requires both old and new admin to
  sign).
- `add_allowed_dst_token` / `set_dst_allowlist_enabled` — controls which tokens
  users can target.

**Implication:** A compromised admin key can grief the protocol (pause, fee
redirection) but **cannot steal user funds or solver bonds directly**, because
no admin function transfers tokens out of user or solver custody.  Key rotation
requires dual authorization, reducing the single-key-failure surface.

#### 3. Destination token allowlist (default: off)

By default, `submit_intent` accepts any `dst_token` address.  If an admin
enables the allowlist (`set_dst_allowlist_enabled(true)`), only pre-approved
tokens may be used as destinations.

**Implication (allowlist disabled):** A user or solver could name a malicious
contract as `dst_token`.  The risk lands on the user who submitted the intent —
they would receive tokens from a contract they did not vet.  Enabling the
allowlist before mainnet is strongly recommended.

**Implication (allowlist enabled):** The allowlist is stored in instance
storage, so adding/removing tokens is cheap but centralized under the admin
key.

#### 4. Fill-window timing

The fill window (`FILL_WINDOW = 300 s`) starts from `accept_intent`.  The
deadline stored on-chain is `now + 300`.  A solver and a block producer
colluding to manipulate `ledger().timestamp()` could extend the fill window
slightly, but Stellar's timestamp drift is bounded by consensus rules and is not
a meaningful attack surface in practice.

#### 5. Permissionless slash and expire

`slash_solver` and `expire_intent` are callable by anyone.  This is intentional
— it prevents a solver from dodging accountability by waiting out a pause or
ensuring only friendly callers settle the intent.  A griefing scenario where
someone repeatedly calls `expire_intent` on an already-expired intent is
defended by the `IntentNotOpen` guard (idempotent after first call).

---

---

### Proof of Reserves

The protocol publishes a **public proof-of-reserves dashboard** (see [`dashboard/`](./dashboard/) and [`docs/proof-of-reserves-dashboard.md`](./docs/proof-of-reserves-dashboard.md)) that reconciles on-chain solver bond totals against this Assets-at-Risk table. This is a **transparency artifact, not an insurance guarantee** — it verifies that collateral is on-chain but does not guarantee solver performance or user fund recovery.

The dashboard is continuously updated and independently reproducible (data sources and queries are published). Users should read this threat model and the FAQ in the dashboard for caveats.

---

### Admin Key Operational Security (#122)

The `Admin` address is the single most sensitive key in the protocol. It
controls pause/unpause, fee routing, the destination-token allowlist, the
source-chain allowlist, and admin rotation. A compromised admin key can halt
the protocol and redirect all fee and slash proceeds. The recommendations below
apply before and after mainnet launch.

**Custody transparency:** See [`docs/custody-transparency.md`](./docs/custody-transparency.md)
for the current custody model (who holds the keys, single-key vs. multisig threshold, and last-verified date).
This document is updated operationally whenever keys are rotated.

#### Recommended custody model

| Deployment stage | Recommended setup |
|-----------------|-------------------|
| Testnet / staging | Developer EOA on a hardware wallet (Ledger/Trezor) |
| Mainnet launch | 2-of-3 or 3-of-5 multisig (e.g. Gnosis Safe equivalent on Stellar, or a Stellar multi-sig account with threshold set appropriately) |
| Long-term mainnet | Hardware-backed multisig with geographically distributed key holders; a minimum M-of-N threshold of M ≥ 2 |

A design for a multisig admin wrapper is tracked in
[`docs/114-multisig-admin-design.md`](./docs/114-multisig-admin-design.md).
Until that wrapper is deployed, at minimum the admin key must reside on a
hardware wallet — never a hot wallet or a key stored in plaintext.

#### Key rotation

- Rotate the admin key with `transfer_admin`. Both the outgoing and incoming
  admin must sign the same transaction, so a typo'd address can never
  accidentally brick the contract.
- After rotation, verify `get_admin()` returns the expected new address on-chain
  before discarding the old key material.
- Rotation should be rehearsed on testnet before any live key handover.

#### Operational hygiene

1. **Hardware wallet required.** The admin signing key must never exist only in
   software (e.g. a `.env` file, a CI secret, or an in-memory key). Use a
   Ledger, Trezor, or equivalent hardware device for all admin transactions.

2. **Separate key per environment.** Testnet, staging, and mainnet must each
   have a distinct admin key. Reusing a key across environments means a
   compromise on a lower-trust environment immediately threatens mainnet.

3. **Minimal admin transactio