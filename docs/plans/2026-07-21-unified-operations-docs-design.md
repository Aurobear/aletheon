# Unified operations entry point and documentation convergence

> Date: 2026-07-21
> Status: proposed

## 1. Requirement and current-state anchors

The SER8 requirement is to run Aletheon as a resident core plus user daemon and
close the scheduled Pi → summarized memory → GBrain loop
(`docs/archive/plans/2026-07-21/2026-07-21-aletheon-ser8-deployment.md:6-7`). The required runtime
topology is a machine core socket below a per-user daemon
(`docs/archive/plans/2026-07-21/2026-07-21-aletheon-ser8-deployment.md:21-29`). The original
operator note separately describes build, installation, provider, Pi, GBrain,
and timer actions (`docs/archive/plans/2026-07-21/2026-07-21-aletheon-ser8-deployment.md:33-127,177-201`).

The closure is accepted in
`docs/deployment/ser8-acceptance-2026-07-21.md:34-68`, but the repository has no
single command surface for repeating that deployment. The installer handles the
binary, core/user units, and machine backup/cleanup timers
(`scripts/install-systemd.sh:30-64,66-84`). The Pi closure wrapper remains a
fixed acceptance workload with a dedicated fixture, source path, provider, and
model (`scripts/aletheon-pi-scheduled-task.sh:6-12,56-90`).

## 2. Requirement versus code/document reality

| Requirement description | Current repository reality | Agree? |
|---|---|---|
| One resident core and per-user daemon (`deployment.md:21-29`) | The native installer installs and verifies both boundaries (`scripts/install-systemd.sh:43-53,66-76`) | Yes |
| Scheduled Pi → memory → GBrain closure (`deployment.md:6-7,186-187`) | The closure assets exist and were accepted, but are not installed by the main installer (`scripts/install-systemd.sh:37-57`; `ser8-acceptance-2026-07-21.md:50-68`) | Partly |
| GBrain is local at `127.0.0.1` (`deployment.md:16,184-185`) | The deployed environment currently uses an explicit Tailscale endpoint; endpoint location is an operator concern, not a portable repository constant | No; user approved local/remote compatibility |
| Pi closed-loop verification is remaining work (`deployment.md:177-187`) | The acceptance record marks Pi, memory, outage recovery, and timer gates PASS (`ser8-acceptance-2026-07-21.md:36-68`) | No; the plan is historical |
| The scheduled runtime uses `pi-rpc` for isolated worktree changes (`deployment.md:181-183`) | The accepted topology assigns shared resident work to `pi-rpc` and isolated diff work to `pi-coder` (`ser8-acceptance-2026-07-21.md:16-23`) | No; preserve the adjudicated dual-runtime model |

The user approved treating the deployment note as history, using the accepted
runtime model, supporting explicit local or remote GBrain endpoints, and
converging the active operational interface and documentation.

## 3. Approaches considered

### A. Expand `install-systemd.sh`

Add build, configuration, health, logs, timer, and deployment behavior to the
existing root installer. This minimizes file count but mixes root and user
operations, makes safe dry runs difficult, and turns a focused installer into a
large stateful command.

### B. Unified dispatcher over focused modules — selected

Follow the aurb pattern: a small `scripts/aletheon.sh` dispatcher sources
focused modules below `scripts/lib/aletheon/`. Existing scripts remain the
low-level compatibility contracts. This creates one discoverable interface
without rewriting reviewed security-sensitive helpers.

### C. Python orchestration CLI

A typed Python command could improve parsing and tests, but it adds a second
runtime implementation style and is unnecessary for the current bounded
systemd orchestration.

## 4. Command design

```text
scripts/aletheon.sh
    |
    +-- build
    +-- install [--no-enable]
    +-- deploy [--no-build] [--no-restart]
    +-- configure show|check
    +-- status
    +-- health
    +-- restart
    +-- logs [core|user|closure]
    +-- verify
    +-- closure install|run|status
    +-- help
```

`deploy` is the repeatable happy path:

```text
preflight -> bounded release build -> native install -> user closure install
          -> daemon reload/restart -> readiness and integration verification
```

The command never extracts or prints secrets. Provider and GBrain credentials
must already exist in the documented restricted environment files. Endpoint
selection is explicit configuration: loopback and Tailscale/remote URLs are
both valid, while missing credentials, invalid URLs, unavailable binaries, or
unsafe file modes fail closed.

Root-only work is delegated through a single visible `sudo` boundary to the
existing installer. User service operations remain unprivileged. Commands that
only inspect state (`status`, `health`, `configure show`, `verify`) do not mutate
services.

## 5. Module boundaries

- `scripts/aletheon.sh`: argument dispatch, help, stable exit codes.
- `scripts/lib/aletheon/common.sh`: repository paths, logging, command checks,
  endpoint validation, and shared error handling.
- `scripts/lib/aletheon/build.sh`: invokes only
  `bash scripts/cargo-agent.sh build -p aletheon --release` with the repository
  target directory.
- `scripts/lib/aletheon/install.sh`: wraps the reviewed native installer and
  installs byte-identical tracked per-user closure assets.
- `scripts/lib/aletheon/service.sh`: status, restart, logs, and timer operations.
- `scripts/lib/aletheon/verify.sh`: repository/deployed artifact comparison,
  systemd verification, socket health, Pi registration evidence, GBrain health,
  and timer state.

No module owns provider tokens. The implementation will prefer existing scripts
and systemd units over new abstractions.

## 6. Documentation convergence

`docs/deployment/README.md` becomes the canonical operator entry point. It
contains prerequisites, configuration locations, local versus Tailscale GBrain
examples, first install, repeat deploy, health/status, timer operation,
upgrade/rollback links, and troubleshooting links. Root `README.md` links to
this guide near the top.

Current reference material remains in its domain:

```text
docs/deployment/README.md       canonical operations entry
docs/deployment/*.md            focused active references
docs/design/                    current design contracts
docs/decisions/                 durable ADRs
docs/testing/                   test strategy and procedures
docs/archive/plans/             superseded implementation histories
```

The dated SER8 deployment/design/implementation notes are moved to
`docs/archive/plans/2026-07-21/` with a short archive README explaining that
they are point-in-time evidence, not current instructions. The final SER8
acceptance record remains under `docs/deployment/`, is corrected for the
portable GBrain topology, and links to the canonical guide. Historical files
are archived rather than deleted; links are updated deterministically.

## 7. Safety and failure behavior

- No secret values in output, argv generated by the tool, repository files, or
  receipts.
- Read-only commands work without root.
- Mutating system operations clearly announce the exact phase before invoking
  `sudo`.
- Existing configuration is preserved unless the operator explicitly replaces
  it, matching `scripts/install-systemd.sh:60-62`.
- A failed build cannot install; a failed install cannot restart; a failed
  readiness gate makes `deploy` non-zero.
- GBrain unavailability reports degraded memory integration without claiming
  the Aletheon daemon is dead.
- Closure installation verifies tracked and installed assets byte-for-byte.

## 8. Validation and acceptance

Automated shell tests cover command dispatch, help, dry/non-mutating status,
endpoint validation, failure propagation, and closure asset staging. Static
validation runs `bash -n` over all changed shell files. Rust validation uses the
repository wrapper only and is limited to affected deployment contracts unless
CI owns workspace-wide validation.

SER8 deployment acceptance for this change requires:

1. `scripts/aletheon.sh help`, `status`, `health`, and `verify` succeed on SER8.
2. `deploy` builds through `scripts/cargo-agent.sh`, installs the exact binary,
   reloads services, and returns only after readiness succeeds.
3. `closure install` produces byte-identical user units/wrapper, and the timer
   remains enabled and active.
4. Both loopback and explicit Tailscale GBrain endpoint forms pass configuration
   validation; the current SER8 endpoint responds successfully.
5. The root README has one obvious deployment link and no active guide sends an
   operator to the archived plans.
6. CI passes before merge to `dev`; the merged `dev` revision is deployed and
   its binary hash is recorded in the final report.

## 9. Out of scope

- Redesigning Pi runtime behavior or auto-applying retained diffs.
- Moving secrets into repository-managed configuration.
- Installing GBrain itself.
- Permanently deleting historical documents in this change.
- Replacing the reviewed backup, restore, cleanup, or upgrade helpers.
