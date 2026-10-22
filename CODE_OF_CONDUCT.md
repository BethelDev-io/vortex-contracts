# Code of Conduct

This document is a **repository-specific supplement** to the org-wide
[CONTRIBUTING.md](https://github.com/vortex-protocol/.github/blob/main/CONTRIBUTING.md).
It does **not** replace or supersede the org-wide code of conduct — it adds
content specific to the risk surface of this repository, which governs a live,
funds-bearing Soroban protocol. Where the two overlap, the org-wide document
remains authoritative for general interpersonal conduct; this document adds the
financial-stakes-specific expectations described below.

---

## Table of Contents

1. [Baseline Interpersonal Conduct](#baseline-interpersonal-conduct)
2. [Responsible-Disclosure Conduct](#responsible-disclosure-conduct)
3. [Conflict of Interest and Privileged Information](#conflict-of-interest-and-privileged-information)
4. [Enforcement and Reporting](#enforcement-and-reporting)

---

## Baseline Interpersonal Conduct

All participants in this repository — contributors, reviewers, maintainers,
arbiters, and admins — are expected to uphold the standard baseline described in
the org-wide [CONTRIBUTING.md](https://github.com/vortex-protocol/.github/blob/main/CONTRIBUTING.md):
be respectful, assume good faith, welcome newcomers, and keep discussion focused
on the work. Harassment, discrimination, and personal attacks are not tolerated
here, exactly as they are not tolerated org-wide.

The sections below do not restate that baseline; they describe additional
expectations that arise because this repository governs real funds.

---

## Responsible-Disclosure Conduct

This repository's [SECURITY.md](SECURITY.md) already defines **how** to report a
vulnerability. This section defines the **conduct expectation** around that
process — the consequence of violating the norm, not a restatement of the
instructions.

- **Do not disclose unpatched vulnerabilities publicly.** `SECURITY.md` states
  that you must not open a public GitHub issue for security vulnerabilities.
  That is a conduct rule, not merely a suggestion. Discussing a not-yet-patched
  vulnerability in a public issue, PR, discussion thread, or social channel is a
  violation of this code of conduct, because on a live, funds-bearing protocol
  public disclosure before a fix ships can directly enable theft of user funds.
- **Follow the private channel.** Report through the private channel described
  in [SECURITY.md](SECURITY.md). If you are unsure whether something is
  sensitive, treat it as sensitive and report privately first.
- **Legitimate research vs. prohibited exploitation.** Security research that
  follows `SECURITY.md` — private report, reasonable coordination window, no
  movement of funds beyond what is needed to demonstrate an issue on a test
  deployment — is welcome and valued. The boundary is crossed when a researcher:
  exploits a vulnerability against mainnet for personal gain, moves or attempts
  to move user funds, extorts the project, or discloses publicly before a fix is
  available. Crossing that boundary is a conduct violation and may also be a
  legal matter; it will be handled accordingly.
- **Respect the coordination window.** Once a report is acknowledged, keep the
  details private until the maintainers confirm a fix has shipped or a
  coordinated-disclosure date is agreed.

---

## Conflict of Interest and Privileged Information

Anyone with elevated authority in this repository — **admins, maintainers,
reviewers, and arbiters** — holds information and powers that ordinary
contributors do not. That asymmetry creates obligations.

### General standard

- **Disclose conflicts.** If you have a personal financial interest that could
  be affected by a change you review, merge, or administer, disclose it and
  recuse yourself from the decision where feasible.
- **Do not use privileged information for personal advantage.** Knowledge gained
  through a privileged role — an unreleased fix, an internal discussion, a
  pending parameter change — must not be used to trade, position, or otherwise
  benefit yourself or anyone close to you before that information is public.
- **Arbiters** are additionally bound by the arbiter-specific code of conduct in
  [`docs/arbiter-code-of-conduct.md`](docs/arbiter-code-of-conduct.md) (issue
  #115). Where that document is more specific about arbiter conflicts of
  interest, it governs; this section is the broader admin/maintainer analogue.

### Privileged knowledge of pending `propose_*` changes

This contract's admin surface uses timelocked `propose_*` entrypoints: a change
is proposed, sits through a timelock window, and only then can be executed. That
window exists precisely so the community can observe and react before a change
takes effect. Conduct expectations during that window:

- **Do not front-run the timelock.** If you learn of a pending `propose_*`
  change through a privileged role before it is publicly visible, you must not
  use that knowledge to trade, adjust positions, or otherwise gain advantage
  ahead of the public.
- **Do not suppress visibility.** Admins and maintainers must not use their
  authority to hide, delay, or obscure a pending proposal from the community
  during its timelock window.
- **Do not leak selectively.** Sharing a pending proposal with a favored party
  before it is public is a violation, even if the eventual change is benign.

### Maintainer/reviewer who is also an active solver

A maintainer or reviewer may also be a registered, actively-bonded solver. This
is not prohibited, but it must be disclosed, and the following applies:

- **Recuse from decisions that touch solver economics.** Do not review, approve,
  or merge changes to bond amounts, slash conditions, fee splits, or fill-window
  logic while you hold an active solver bond that those changes would affect.
  Hand the review to an uninvolved maintainer.
- **Do not use review access to gain solver advantage.** Reading an unreleased
  change to fill-window or slashing logic and adjusting your own solver behavior
  before it ships is a violation of this code.
- **Disclose the dual role** in the PR or issue where the conflict is relevant,
  so other maintainers can route the decision appropriately.

---

## Enforcement and Reporting

- **Where to report conduct concerns.** Report interpersonal-conduct concerns
  through the org-wide process described in the org-wide
  [CONTRIBUTING.md](https://github.com/vortex-protocol/.github/blob/main/CONTRIBUTING.md).
  Report **security-sensitive** conduct concerns (public disclosure of an
  unpatched vulnerability, exploitation, privileged-information abuse) through
  the private channel in [SECURITY.md](SECURITY.md) so the report itself does
  not become a disclosure.
- **What happens next.** Maintainers will review the report privately, give the
  accused an opportunity to respond, and decide on a response proportionate to
  the risk. Because this repository governs real funds, responses to
  financial-stakes violations (exploitation, front-running a timelock, leaking
  privileged information) may be more severe than for ordinary interpersonal
  issues — up to and including removal of commit/review/admin/arbiter authority
  and a public statement once any live vulnerability is patched.
- **No retaliation.** Retaliating against someone who reports a concern in good
  faith is itself a violation of this code.
- **Good-faith mistakes.** An honest mistake — e.g. mentioning a suspicious
  behavior in the wrong channel before realizing it was sensitive — handled
  promptly and privately is treated as a learning opportunity, not a violation.

---

## Relationship to Other Documents

- General contributor process and interpersonal conduct: org-wide
  [CONTRIBUTING.md](https://github.com/vortex-protocol/.github/blob/main/CONTRIBUTING.md).
- Vulnerability reporting instructions: [SECURITY.md](SECURITY.md).
- Arbiter-specific conduct: [`docs/arbiter-code-of-conduct.md`](docs/arbiter-code-of-conduct.md)
  (issue #115).

This document supplements those; it does not supersede them.
