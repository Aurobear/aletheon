# Generic Subagent Runtime Acceptance

**Date:** 2026-07-26
**Source commit:** `8430433d` plus profile-compatibility commit `1d6e2b7`
**Track:** `aletheon-tester` real-TUI track using `/usr/bin/aletheon`
**Verdict:** PARTIAL — functional behavior passed; strict provider-error gate failed

## Installed provenance

- `target/release/aletheon`: `a1315ad61f7a0e0724515b960557078858c07c351ae5ffad825922c5f7395f82`
- `/usr/bin/aletheon`: `a1315ad61f7a0e0724515b960557078858c07c351ae5ffad825922c5f7395f82`
- machine-core executable: `a1315ad61f7a0e0724515b960557078858c07c351ae5ffad825922c5f7395f82`
- user-daemon executable: `a1315ad61f7a0e0724515b960557078858c07c351ae5ffad825922c5f7395f82`
- machine core: active, `NRestarts=0`
- user daemon: active, `NRestarts=0`

System deployment completed successfully with:

```text
sudo bash scripts/aletheon.sh deploy
```

No user-local Aletheon binary was deployed.

## Three consecutive real-TUI runs

Each run used a fresh TUI from the repository root and requested:

```text
profile=researcher
runtime omitted
required_capabilities=[code_read, code_search]
read-only README analysis
```

| Run event ID | Selected runtime | Tools | Inference rounds | Input tokens | Output tokens | Prompt returned |
|---|---|---:|---:|---:|---:|---|
| `efd7e1bced3b49d9b119f6cfc418a037` | `pi-rpc` | 2 | 3 | 20,888 | 538 | yes |
| `3abfee5b624d4825b0f8024b0e0595db` | `pi-rpc` | 2 | 3 | 21,030 | 538 | yes |
| `30f218a31667447b82175a7a94520aa5` | `pi-rpc` | 2 | 3 | 21,060 | 507 | yes |

All three rendered substantive final answers. Each used exactly one
`agent_spawn` and one `agent_wait`. Executive logs recorded
`override_used=false` and effective capabilities `[CodeRead, CodeSearch]`.

## Failed strict gate

The second run logged two provider retry attempts:

```text
Streaming inference unavailable; retrying attempt=1 ... provider_unavailable
Streaming inference unavailable; retrying attempt=2 ... provider_unavailable
```

No provider error was rendered in the final TUI frame and all three tasks
succeeded, but repository acceptance policy treats any such daemon-log marker
as a failed real-TUI run. Therefore this evidence must not be reported as full
installed acceptance.

The generic selection and lifecycle behavior is accepted by deterministic
tests and observable TUI results. Provider request pacing/retry behavior remains
a separate unresolved acceptance blocker.
