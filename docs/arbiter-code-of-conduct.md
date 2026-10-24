# Arbiter Code of Conduct

This document defines the conduct expected of anyone serving as an **arbiter** in this repository's dispute-resolution process. It supplements the org-wide `CONTRIBUTING.md` code-of-conduct reference and the repo-specific `CODE_OF_CONDUCT.md`; it does not replace either. Where this document is silent, those documents govern.

Arbiters hold real financial-decision authority over a live, funds-bearing protocol. The standards below exist because that authority is not abstract: an arbiter's ruling can move funds, and an arbiter's privileged knowledge can be worth money to the wrong person.

## 1. Baseline conduct

Arbiters are held to the same interpersonal-conduct baseline as every contributor (harassment, inclusivity, good-faith engagement). Holding arbiter authority does not exempt anyone from that baseline, and it does not grant license to be harsher in ordinary community interactions than any other contributor.

## 2. Impartiality and conflicts of interest

- **Disclose conflicts.** Before participating in a dispute, an arbiter must disclose any relationship to a party that a reasonable observer could see as compromising: being a party's counterparty, a registered/actively-bonded solver on the same market, a delegate, an employee, or a close collaborator.
- **Recuse when conflicted.** An arbiter with a material conflict must recuse from that dispute rather than rule on it. "Material" means the outcome could plausibly affect the arbiter's own funds, position, or obligations.
- **No self-dealing.** An arbiter must not rule on a dispute in which they are a party, and must not use arbiter authority to advantage a position they hold.

## 3. Privileged information

Arbiters routinely see information before the rest of the community does: dispute filings, evidence, and the timing of pending `propose_*` changes during their timelock windows. Using that information for personal advantage — trading ahead of a pending change, tipping off a counterparty, or front-running a resolution — is prohibited.

- **Do not act on non-public information.** Knowledge gained through arbiter duties must not inform any position, trade, or coordination the arbiter undertakes in their personal capacity.
- **Respect timelock windows.** A pending `propose_*` change is not public knowledge merely because an arbiter can see it. Discussing or acting on it outside the appropriate channel before it is public is a violation.
- **Keep deliberations confidential.** Dispute deliberations and evidence remain confidential until the resolution is published through the normal process.

## 4. Responsible disclosure

Arbiters are often the first to learn of a vulnerability or an exploit in progress. The responsible-disclosure norm in `SECURITY.md` applies to arbiters with full force: do not open a public issue, do not discuss the vulnerability in public channels, and route reports through the process `SECURITY.md` describes. An arbiter who learns of an unpatched vulnerability must treat it as confidential and must not use it, or the knowledge of it, for personal advantage.

## 5. Enforcement and reporting

Concerns about an arbiter's conduct — a failure to disclose a conflict, use of privileged information, or a disclosure violation — should be reported privately to the maintainers/admin committee through the private channel described in `SECURITY.md` (or the maintainer contact listed there). Reports are handled confidentially. Confirmed violations may result in removal from arbiter duties and, where warranted, further action under the repo-specific `CODE_OF_CONDUCT.md`.

## 6. Relationship to other documents

- `SECURITY.md` — responsible-disclosure reporting instructions (referenced, not restated, here).
- `CODE_OF_CONDUCT.md` — repo-specific conduct supplement covering this protocol's financial-stakes dynamics.
- Org-wide `CONTRIBUTING.md` — general contributor process and baseline code-of-conduct reference.

This document is scoped to arbiter conduct. It does not supersede the org-wide code of conduct; it adds the commitments that arbiter authority specifically requires.
