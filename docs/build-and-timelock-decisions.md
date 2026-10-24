# Build reproducibility and timelock adoption

## Cargo.lock policy (#553)

Only `intent_settlement`, `proof_registry` and `solver_registry` commit a
`Cargo.lock` (the root `Cargo.lock` covers the workspace members;
`backstop_vault` and `intent_periphery` are explicitly git-ignored). Crates
built standalone (`cd <crate> && cargo build`, as in the `Makefile` and
`release.yml`) without a lockfile resolve dependencies fresh, so the same
source can yield different wasm checksums.

Options for a maintainer decision:

1. **Per-crate lockfiles** — run `cargo generate-lockfile` in each standalone
   crate, commit the result, and remove its entry from `.gitignore`.
2. **Workspace lock only** — add every crate to the root `[workspace]` members
   (see #1), build with `cargo build --locked` from the root, and treat the
   root `Cargo.lock` as the single source of truth.

Until one is chosen, release builds should pass `--locked` where a lockfile
exists.

## Standalone `timelock` (#550)

`timelock` is currently not called by any contract: `governor` stores its
address but never invokes it (#5), and `intent_settlement`, `solver_registry`,
`fee_router` and `treasury_streams` each implement their own propose/delay/
execute logic. Whether to migrate them to the shared contract or keep the
duplication (avoiding cross-contract-call overhead) is an open architectural
decision; migrations, if any, should be filed as per-contract follow-ups.

For local testing the contract is deployable with:

```bash
./setup-local-network.sh deploy-timelock <admin_address> [min_delay_secs]
```
