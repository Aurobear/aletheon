# Aletheon operations guide

This is the canonical entry point for building, installing, deploying, and
checking a native Aletheon host. Dated files below `docs/archive/` preserve
historical decisions and evidence; do not use them as current runbooks.

## Runtime topology

```text
TUI / scheduled closure
        |
        v
user aletheon.socket -> aletheon.service
        |                    |
        |                    +-> Pi RPC / Pi coder / local memory / GBrain
        v
/run/aletheon/core.sock -> aletheon-core.service -> configured provider
```

The machine core and each user's daemon have separate configuration, credentials,
and systemd authority. See [systemd.md](systemd.md) for the security boundary and
[secrets.md](secrets.md) for credential handling.

## Prerequisites

- Linux with systemd user managers
- Rust toolchain matching `rust-toolchain.toml`
- `bash`, `python3`, `git`, `curl`, `bubblewrap`, `sqlite3`, and build essentials
- reviewed Pi executable when `[pi_runtime]` is enabled
- provider and optional GBrain credentials in external mode-restricted files

Never place API keys or bearer tokens in this repository, command arguments,
unit text, prompts, or evidence files.

## Unified command

Run all operations from the repository root:

```bash
bash scripts/aletheon.sh help
bash scripts/aletheon.sh configure show
bash scripts/aletheon.sh configure check
bash scripts/aletheon.sh status
bash scripts/aletheon.sh health
bash scripts/aletheon.sh verify
```

The interface follows a dispatcher/module layout: `scripts/aletheon.sh` is the
only operator-facing entry point and focused implementations live below
`scripts/lib/aletheon/`. Existing install, health, backup, restore, cleanup, and
upgrade scripts remain low-level reviewed contracts.

## Configuration

Machine core configuration is `/etc/aletheon/config.toml`. User daemon
configuration is normally `~/.aletheon/config.toml`. Provider environment files
are external to the repository:

```text
/etc/aletheon/credentials/provider.env    machine core provider
~/.config/aletheon/daemon.env             user daemon and Pi provider
~/.config/aletheon/gbrain.env             optional GBrain bearer token
```

The GBrain MCP endpoint is selected in the user config, not hard-coded by the
operations tool. Both of these deployments are valid:

```toml
# Same-host service
[[mcp_servers]]
name = "gbrain"
url = "http://127.0.0.1:3131/mcp"
trust = "LocalTrusted"

# Tailnet-reachable service
[[mcp_servers]]
name = "gbrain"
url = "http://100.x.y.z:3131/mcp"
trust = "RemoteTrusted"
```

Use HTTPS for networks that are not already protected by a trusted private
transport. `configure check` accepts only credential-free HTTP(S) URLs and
rejects file URLs or URLs containing user information.
`LocalTrusted` is loopback-only. `RemoteTrusted` permits non-loopback private
addresses (for example a tailnet) but still rejects loopback, link-local and
metadata addresses; use `Untrusted` for public-address-only MCP endpoints.

## First installation

Review `config/production.toml.example`, provision external credentials, then:

```bash
bash scripts/aletheon.sh build
bash scripts/aletheon.sh install --no-enable
sudo usermod -aG aletheon "$USER"
```

Log out and back in so the user manager receives the new supplementary group.
Then enable the user socket and install the scheduled closure:

```bash
systemctl --user enable --now aletheon.socket
bash scripts/aletheon.sh closure install
bash scripts/aletheon.sh restart
bash scripts/aletheon.sh verify
```

For image construction, `--no-enable` installs and verifies assets without
starting the system services.

## Repeat deployment

After credentials and configuration are established, the complete repeatable
path is:

```bash
bash scripts/aletheon.sh deploy
```

It runs these fail-fast phases:

```text
bounded release build
  -> native system install
  -> byte-identical user closure install
  -> core and user daemon restart
  -> complete deployed-state verification
```

Useful controlled variants:

```bash
bash scripts/aletheon.sh deploy --no-build
bash scripts/aletheon.sh deploy --no-restart
bash scripts/aletheon.sh deploy --no-enable
```

## Runtime operations

```bash
bash scripts/aletheon.sh status
bash scripts/aletheon.sh health
bash scripts/aletheon.sh restart
bash scripts/aletheon.sh logs user
bash scripts/aletheon.sh logs core
bash scripts/aletheon.sh logs closure
```

`health` checks the core socket, the private user daemon RPC, and the configured
GBrain health endpoint. `verify` additionally checks service/timer activation,
installed closure asset identity, and Pi runtime registration evidence.

## Scheduled Pi-memory closure

```bash
bash scripts/aletheon.sh closure install
bash scripts/aletheon.sh closure status
bash scripts/aletheon.sh closure run
```

The tracked timer runs daily with randomized delay. Its current task is a
bounded operational fixture, not an unrestricted auto-apply job. It never
applies retained Pi diffs automatically. Review any retained diff before using
the approved-apply path.

## Upgrade, rollback, and recovery

- Follow [upgrade-rollback.md](upgrade-rollback.md) for binary rollout and rollback.
- Follow [backup-restore.md](backup-restore.md) and
  [disaster-recovery.md](disaster-recovery.md) for state recovery.
- Follow [observability.md](observability.md) for health and journal signals.
- Follow [operations-checklist.md](operations-checklist.md) for production review.

After every upgrade or rollback, run:

```bash
bash scripts/aletheon.sh verify
```

## Common failures

| Symptom | Check | Resolution |
|---|---|---|
| Core socket permission denied | `id -nG`, `/run/aletheon/core.sock` | Add the user to `aletheon`, then refresh the login/user manager |
| GBrain MCP initialization fails | `bash scripts/aletheon.sh configure show` | Select the reachable loopback or tailnet endpoint and verify its service binding |
| Pi runtime is absent | `bash scripts/aletheon.sh logs user` | Verify Pi executable hash, fixed arguments, namespace support, and allowed paths |
| Closure timer exists but does not run | `bash scripts/aletheon.sh closure status` | Re-run `closure install`, then inspect closure logs |
| Deploy stops before restart | The first failing phase in output | Correct that phase; deploy intentionally does not continue after failure |
