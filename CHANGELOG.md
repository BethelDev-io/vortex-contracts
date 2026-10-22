# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Changed

- Upgraded `soroban-sdk` from `21.0.0` to `22.0.0` across the workspace root and every contract crate (`intent_settlement`, `proof_registry`, `solver_registry`, `reputation_badge`, `backstop_vault`), closing the gap left by issue #377. Migration checklist followed from `docs/event-schema.md` (typed events, test registration API, TTL/auth API notes).
