# Installed Runtime Verification Design

## Status

Approved approach: strict production deployment gate.

## Problem

Development checks can pass against a repository debug binary or an isolated
daemon while the host continues to run an older `/usr/bin/aletheon`. Agent
profiles, tool registries, configuration, state migrations, and protocol
contracts are validated at runtime, so source-level and isolated tests cannot
prove that the installed system is usable.

This mismatch caused an installed user daemon restart loop: a profile referenced
the new `robot_observe` tool while `/usr/bin/aletheon` still registered the old
`robot.observe` name. The isolated test runtime passed, but it was not valid
deployment evidence.

## Goals

1. Make the installed release binary the only acceptable final acceptance
   target.
2. Prove that the release candidate, installed binary, and running daemons are
   the same artifact.
3. Detect startup validation failures and restart loops after deployment.
4. Exercise the official client, official user socket, installed daemon, and
   configured LLM provider with a real request.
5. Fail closed: a deployment is not complete until every installed-runtime gate
   passes.

## Non-goals

- Replacing narrow unit, integration, simulator, or direct gRPC tests.
- Requiring a real provider during ordinary development builds.
- Treating optional GBrain availability as proof that the core runtime works.
- Automating an interactive terminal UI with synthetic keystrokes.

## Required Acceptance Boundary

```text
repository source
      |
      v
bounded release build (scripts/cargo-agent.sh)
      |
      v
target/release/aletheon
      |
      v
native install -> /usr/bin/aletheon
      |
      v
official systemd core + per-user daemon
      |
      v
official user socket
      |
      v
/usr/bin/aletheon single-message client
      |
      v
configured real LLM provider -> completed response
```

Debug binaries, temporary homes, alternative sockets, isolated daemons, direct
provider probes, and direct bridge calls remain useful diagnostic evidence, but
they never satisfy final acceptance.

## Design

### 1. Repository operating constraint

`AGENTS.md` will define an installed-runtime verification policy:

- Repository Rust commands continue to use `scripts/cargo-agent.sh`.
- Final acceptance must use `bash scripts/aletheon.sh deploy`.
- Test reports must distinguish development evidence from installed-runtime
  evidence.
- A change affecting tools, profiles, configuration, persistence, IPC, daemon
  bootstrap, or client behavior cannot be called complete without the strict
  deployment gate.
- Test-only profiles and assets must use isolated state roots and must never be
  copied into the active user state unless the matching release is installed.

### 2. Binary provenance gate

After installation and restart, verification computes SHA-256 for:

- `target/release/aletheon`;
- `/usr/bin/aletheon`;
- the executable referenced by the running user daemon;
- the executable referenced by the running machine core.

All hashes must equal the release candidate hash. Verification also checks that
the user service command resolves to `/usr/bin/aletheon`, preventing a passing
health check from an unintended debug or temporary process.

The gate fails with the expected and observed paths and hashes, without exposing
credentials or dumping process environments.

### 3. Runtime stability gate

Verification records the user daemon and core service identity and restart
counters, waits for a bounded stability interval, then checks again:

- both services remain active;
- the service identities remain valid;
- restart counters do not increase;
- recent service logs contain no fatal startup/profile validation error.

The default interval is short enough for local deployment while exceeding the
normal rapid-restart cadence. Tests may override it through an explicitly named
environment variable.

### 4. Official-client real-request gate

The deployment gate invokes the installed client, not the repository binary:

```bash
/usr/bin/aletheon -m "<bounded smoke prompt>"
```

The client uses the official user socket and active user configuration. The
smoke prompt asks for a short deterministic textual response and does not grant
tools or perform hardware actions. Success requires:

- client exit status zero;
- a non-empty completed response;
- no daemon restart during the request;
- final user-daemon health remains ready or explicitly accepted degraded state
  under the existing health policy.

The prompt and timeout are configurable for controlled deployments, but the
gate cannot be silently skipped by the default `deploy` command. A deliberately
offline installation must use an explicit deployment variant and must not be
reported as full production acceptance.

### 5. Command behavior

`bash scripts/aletheon.sh deploy` becomes:

```text
build
  -> install
  -> install closure assets
  -> restart official services
  -> configuration and socket health
  -> binary provenance
  -> runtime stability
  -> official-client real request
  -> final provenance and health
```

`bash scripts/aletheon.sh verify` runs the same installed-state gates without
rebuilding or installing. Existing `--no-build`, `--no-restart`, and
`--no-enable` variants remain controlled operational tools, but verification
still rejects stale or mismatched installed artifacts.

### 6. Documentation

The deployment guide and operations checklist will state:

- the supported build command;
- the single repeat-deployment command;
- the difference between development and installed-runtime evidence;
- how to inspect provenance and stability failures;
- that manually copying profiles into active user state is forbidden;
- that deployment is incomplete if the real request cannot run.

## Failure Semantics

| Failure | Deployment result | Operator action |
|---|---|---|
| Release and installed hashes differ | Fail | Reinstall the candidate |
| Running executable hash differs | Fail | Restart the correct systemd unit |
| Service restart counter increases | Fail | Inspect startup validation logs |
| Profile references unknown tools | Fail | Deploy matching binary/profile set |
| Official client cannot connect | Fail | Repair official socket/service |
| LLM request fails or times out | Fail | Repair provider configuration/network |
| Optional GBrain is unavailable | Existing health policy | Report separately; do not misattribute |

No failure automatically deletes user state, profiles, or rollback evidence.

## Test Strategy

Shell-level tests will run with temporary fake binaries, sockets, command
wrappers, and systemd/journal fixtures. Required cases:

1. matching candidate, installed, and runtime hashes pass;
2. installed binary mismatch fails;
3. runtime executable mismatch fails;
4. restart counter increase fails;
5. fatal profile validation evidence fails;
6. official-client request failure or empty output fails;
7. complete official-client request passes;
8. diagnostics do not print secrets.

Repository Rust validation remains narrow and uses
`bash scripts/cargo-agent.sh`. Final manual evidence must include the candidate
hash, installed hash, runtime hashes, service stability result, and real-request
result.

## Acceptance Criteria

- `AGENTS.md` contains the installed-runtime hard constraint.
- `deploy` fails when repository, installed, and running binaries diverge.
- `deploy` detects the daemon restart-loop scenario that motivated this design.
- `deploy` completes a real request through `/usr/bin/aletheon` and the official
  socket.
- `verify` reproduces all post-install gates.
- Deployment documentation contains one canonical build/deploy/verify workflow.
- Development-only evidence is never labeled as final deployment acceptance.
