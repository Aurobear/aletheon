# APX/CGP workspace-trust host cutover — 2026-08-13

The pure workspace-trust decision contracts remain in Application. The host
orchestration—bounded Git metadata discovery, filesystem receipt store,
canonical event projection, resolver and approval flow—is now owned by the
Aletheon composition tree at `crates/aletheon/src/wiring/workspace_trust.rs`.
All Aletheon production callers use that binary-owned host adapter directly,
removing the Executive path from production.

Validation:

- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: passed.
