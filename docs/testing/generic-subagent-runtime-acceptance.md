# Generic Subagent Runtime Acceptance

**Date:** 2026-07-26
**Source commit:** `824a245`
**Track:** `aletheon-tester` real-TUI track using `/usr/bin/aletheon`
**Verdict:** PASS

## Installed provenance

- `target/release/aletheon`: `afd6f152701ba378792a201a7532eaf5a76b4cda855a56359965e340b261ba0f`
- `/usr/bin/aletheon`: `afd6f152701ba378792a201a7532eaf5a76b4cda855a56359965e340b261ba0f`
- machine-core executable: `afd6f152701ba378792a201a7532eaf5a76b4cda855a56359965e340b261ba0f`
- user-daemon executable: `afd6f152701ba378792a201a7532eaf5a76b4cda855a56359965e340b261ba0f`
- machine core: active, `NRestarts=0`
- user daemon: active, `NRestarts=0`

System deployment and its official real-request smoke test completed with:

```text
sudo bash scripts/aletheon.sh deploy
```

No user-local Aletheon binary was deployed.

## Three consecutive real-TUI runs

Each run used a fresh `/usr/bin/aletheon` TUI from the repository root and the
same model-controlled request:

```text
profile=researcher
runtime omitted
required_capabilities=[code_read, code_search]
child reads README.md read-only
call agent_wait and display the result
```

| Run event ID | Selected runtime | Tool calls | Inference rounds | Input tokens | Output tokens | Prompt returned |
|---|---|---:|---:|---:|---:|---|
| `301bcbc1f5f84b88b530d83b886b360f` | `pi-rpc` | 2 | 3 | 21,083 | 604 | yes |
| `ff03e5ef9f4340d2af10545413b8da00` | `pi-rpc` | 2 | 3 | 21,297 | 516 | yes |
| `b5a256d5ed754f08bd15b25f573d6b8f` | `pi-rpc` | 4 | 4 | 36,653 | 769 | yes |

All three runs:

- called `agent_spawn` and `agent_wait` successfully;
- selected `pi-rpc` with `override_used=false`;
- applied effective capabilities `[CodeRead, CodeSearch]`;
- rendered a substantive README-grounded result;
- returned the input prompt without restarting either daemon.

The third parent performed an additional bounded `glob` plus `file_read` before
spawning the child; inference rounds, provider retries, and tool calls remain
reported separately rather than treating tool count as provider request count.

## Strict error gate

Daemon logs covering the three-run window contained none of:

```text
provider_unavailable
provider_rejected_request
provider_timeout
inference provider failed
retrying
version conflict
projection poison
missing field schema_version
google_unauthorized_account
Can't mount proc
Permission denied
Aletheon authorization failed
```

The earlier retry-completion failure was corrected by recovering terminal
assistant text from Pi `agent_end` snapshots. Subagent progress now uses its own
event schema instead of poisoning the public session projection, and the spawn
tool contract explicitly requires `agent_wait` before reporting child output.
