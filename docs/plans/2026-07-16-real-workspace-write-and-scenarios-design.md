# Real Workspace Writes and Scenario Tests Design

## Problem

The deployed daemon runs with `ProtectSystem=strict`, `ProtectHome=read-only`,
and only `/var/lib/aletheon`, `/var/cache/aletheon`, and `/run/aletheon` in
`ReadWritePaths`. Consequently `/home/aurobear/Bear-ws` is read-only before a
tool reaches Bubblewrap. Bubblewrap's writable bind of the validated client
working directory cannot override the outer systemd mount namespace, so both
`file_write` and shell writes fail with `EROFS`.

Existing unit tests validate command arguments and in-process behavior but do
not prove that the installed binary, production unit, daemon user, namespace
stack, real TUI, and host filesystem cooperate. A green test suite therefore
does not establish that Aletheon can deliver a file in an actual project.

## Permission boundary

The systemd unit exposes `/home/aurobear/Bear-ws` as a potential writable
source. This is necessary for a nested Bubblewrap bind to be writable. The
daemon continues to run as the dedicated `aletheon` user with strict system
protection.

Each tool request still receives a canonical client working directory. The
runtime accepts only directories beneath the configured workspace root or the
legacy state root. Bubblewrap exposes only that validated working directory as
writable; the rest of the filesystem stays read-only and network isolation is
unchanged. Repository metadata and credential paths inside the writable tree
are re-protected or masked, including `.git`, `.env`, private keys, and OAuth
material.

The effective boundary is:

```text
systemd potential source: /home/aurobear/Bear-ws
                         |
validated client cwd ----+
                         |
bubblewrap writable: exactly <client cwd>
protected inside cwd: .git + secrets
```

## Real scenario suite

A new deployment scenario runner records source commit, installed binary hash,
systemd properties, daemon start time, TUI frame, session JSONL, journal
errors, tool events, and host-side artifacts. It must fail if it cannot
reconcile source and deployed provenance.

Scenarios:

1. **Repository analysis** — launch the real TUI in the project, inspect actual
   manifests and source, and require a substantive final answer without false
   Git or sandbox errors.
2. **Artifact delivery** — request a uniquely named Markdown file under a
   temporary project `docs/plans`, then verify its exact content and ownership
   from the host.
3. **Workspace boundary** — prove writes inside the selected project succeed
   while writes to a sibling project, `.git`, `/etc`, and credential-like paths
   fail.
4. **Git awareness** — require correct branch, HEAD, and dirty-state reporting
   without global Git configuration advice.
5. **Google read path** — use the bound Gmail account in a read-only query and
   require authorization plus a structured summary; never mutate mail.
6. **Restart recovery** — restart the daemon between turns and verify workspace
   identity and durable account/session state remain usable.
7. **Long-turn completion** — exercise at least ten successful tool calls and
   require a final answer after the last result.
8. **TUI stress** — render long Chinese/Markdown output, tables, tool failures,
   repeated PageUp/PageDown input, and terminal exit/re-entry without corruption
   or material scroll latency regression.

Model-controlled scenarios run three consecutive times. Completion requires an
authoritative `turn_done` event associated with the TUI session, not response
length, fixed sleeps, or a merely visible prompt.

## Test layers

- Unit tests cover systemd template assertions, canonical path admission,
  Bubblewrap bind order, protected subpaths, and scenario assertion logic.
- Integration tests execute Bubblewrap against temporary real directories and
  verify host-visible writes plus denied protected writes.
- Deployment tests install the exact release binary and unit, restart the
  daemon, and execute the real-TUI scenarios.

Unit and integration tests may run in CI. Gmail and systemd deployment
scenarios are explicitly environment-gated but are mandatory before reporting
a local production deployment healthy.

## Failure handling

Every scenario reports PASS, FAIL, or BLOCKED with evidence. A tool error,
missing final answer, absent artifact, provenance mismatch, leaked secret,
write outside the selected workspace, or journal warning is a failure. Missing
external credentials is BLOCKED rather than silently skipped. Test artifacts
use unique temporary directories and are removed only after evidence is
captured.

## Scope

This change fixes deployed workspace writes and builds a production scenario
suite. It does not grant unrestricted home-directory writes, permit direct
`.git` mutation, enable network access for generic shell tools, or treat Gmail
read access as permission to send or modify messages.
