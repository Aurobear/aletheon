# SER8 hardening deployment acceptance — 2026-07-22

## Scope

This record closes the post-hardening SER8 deployment gate required by
`docs/plans/2026-07-21-production-readiness-hardening.md:350-360`. It supplements,
rather than replaces, the full functional acceptance in
`docs/deployment/ser8-acceptance-2026-07-21.md`.

## Deployed revision

| Artifact | Evidence |
|---|---|
| Git revision | `1c8320545fa641015f4b084d06799486612748f2` |
| `/usr/bin/aletheon` | SHA-256 `3c6b009becf57a8620562b53b40d81cb8fcaa3f6e8fe96549639acb851454457` |
| Release input | `target/release/aletheon` had the identical SHA-256 |

The deployment used the canonical repeat-deployment entry point:

```text
bash scripts/aletheon.sh deploy
```

The build and installation phases completed, then only the Aletheon machine core
and user daemon were restarted. The host was not rebooted.

## Runtime gate

The first verification correctly failed closed because the tailnet GBrain URL
still had the default `LocalTrusted` trust class. The endpoint itself was healthy,
but the MCP outbound policy rejected a non-loopback address. The user configuration
was corrected to the documented pairing:

```toml
url = "http://100.120.122.46:3131/mcp"
trust = "RemoteTrusted"
```

After the scoped service restart, `bash scripts/aletheon.sh verify` passed:

| Gate | Result |
|---|---|
| Configuration paths and endpoint syntax | PASS |
| Machine core | `ready` |
| User daemon | `alive`, `ready` |
| Local memory and GBrain spool | `ready` |
| GBrain health | `ok`, version `0.42.59.0`, engine `pglite` |
| Pi runtimes | `pi-coder` and `pi-rpc` registered |
| Scheduled closure | user timer installed, enabled and active |

The local workspace regression gate also passed before deployment:

```text
bash scripts/cargo-agent.sh test --workspace -- --test-threads=1
```

All tests and doctests completed with exit status 0. The PR CI matrix independently
passed architecture fitness, source policy, deterministic coding evidence, Linux
Platform contract, Rust 1.88 MSRV, check, test, pinned Pi RPC contract, Clippy,
formatting, fuzz quick-check, docs and release build.

## Acceptance

H0–H11 and the post-change SER8 deployment gate are accepted on this host. Disabled
optional integrations remain explicitly reported as disabled; enabled core, local
memory, GBrain and Pi paths are ready. No database rollback or host reboot was
required.
