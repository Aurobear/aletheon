# H8 Runtime Path Classification and Migration — 2026-07-22

## Requirement anchors

- `/tmp`、socket、cache、worktree、测试 tempdir 必须逐点分类，不能按字符串机械迁移；持久数据、
  runtime、cache 与 secret 必须按生命周期分开：
  `docs/plans/2026-07-21-production-readiness-hardening.md:248-252`。
- 每个生产路径必须有 owner/生命周期/权限/cleanup，多实例不碰撞，迁移保留兼容说明：
  `docs/plans/2026-07-21-production-readiness-hardening.md:254-258`。

## Production path contract

| Class | Owner and canonical root | Lifecycle / permission | Cleanup |
|---|---|---|---|
| machine state | service user, `/var/lib/aletheon/{state,goals,sessions,mnemosyne,artifacts,worktrees,audit}` | persistent, installer `0750` | only marker-authorized cleanup for worktrees/sessions/artifacts |
| machine runtime | service user, `/run/aletheon` | boot/session lifetime, installer `0750`; socket/PID colocated | socket/PID removed on shutdown; `/run` cleared by OS |
| machine cache | service user, `/var/cache/aletheon` | disposable, installer `0750` | marker-authorized cleanup |
| machine secrets | root + `aletheon` group, `/etc/aletheon/credentials` | persistent, files `0600` | explicit rotation only, never generic cleanup |
| user state | invoking user, `$XDG_DATA_HOME/aletheon` (HOME fallback) | persistent, user root prepared `0700` | owner-specific migration/retention |
| user runtime | invoking user, `$XDG_RUNTIME_DIR/aletheon` | login lifetime, root prepared `0700` | daemon sidecars removed; OS clears XDG runtime |
| user cache | invoking user, `$XDG_CACHE_HOME/aletheon` | disposable, user root prepared `0700` | producer retention; entire cache may be removed while stopped |

Machine path definitions and validation are centralized in
`crates/fabric/src/types/paths.rs:6-15,229-319`; ownership/mode creation is implemented by
`scripts/install-systemd.sh:16-27`, and safe marker cleanup by `scripts/cleanup-aletheon.sh:4-43`.
User path resolution requires XDG runtime and resolves state/cache with HOME fallbacks
(`crates/fabric/src/types/paths.rs:34-88`). H8 also makes legacy XDG helpers honor
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, and `XDG_CACHE_HOME`, with a fail-closed non-existent path
instead of `/tmp` when HOME is absent (`crates/fabric/src/types/paths.rs:423-495`).

## Corrected production paths

- Legacy Interact CLI now resolves the same per-user socket as the main binary and passes that exact
  socket to start/status/stop (`crates/interact/src/host.rs:26-46`、
  `crates/interact/src/tui/cli.rs:220-258,395-445`). Explicit `--socket` retains machine-socket
  compatibility.
- Daemon PID and embedded MCP socket are siblings of the configured daemon socket, not shared
  `/tmp` names. Relative sockets fail closed; PID is `0600`; different runtime roots yield different
  sidecars (`crates/executive/src/host/mod.rs:64-70,109-136,179-190`、
  `crates/executive/src/host/systemd.rs:140-150`).
- Bootstrap working directory now uses configured path or current directory; inability to resolve
  either fails startup instead of silently authorizing `/tmp`
  (`crates/executive/src/core/runtime_core.rs:73-84`).
- Tool overflow is disposable user cache, not `/tmp`; filenames contain PID + timestamp + process
  sequence, are created atomically with `0600`, and enforce the existing seven-day retention on
  overflow writes (`crates/corpus/src/tools/tools/output/config.rs:48-60`、
  `crates/corpus/src/tools/tools/output/persistence.rs:39-137`).
- LanceDB defaults to XDG data. An existing legacy `~/.aletheon/vector-db` remains readable only when
  the canonical directory does not yet exist (`crates/mnemosyne/src/impl/vector_store.rs:41-62`).

## `/tmp` classification

- Rust `tempfile::tempdir()` and `/tmp` literals under `#[cfg(test)]` remain isolated fixtures and were
  intentionally not migrated.
- release/multi-user scripts use `mktemp -d` plus traps for disposable acceptance workspaces; these
  are test artifacts, not runtime defaults (`scripts/verify-multi-user-runtime.sh:99`、
  `scripts/release-acceptance.sh:268-377`).
- GBrain container `/tmp` is an isolated `noexec,nosuid,nodev` tmpfs, not host persistent state
  (`deploy/gbrain/compose.yaml:25`).
- No production secret is introduced into ordinary temporary storage. Tool output cache may contain
  sensitive model/tool text, so files are owner-only `0600` even though the cache is disposable.

## Migration and rollback

1. Stop the affected user daemon (do not reboot the computer).
2. For LanceDB, move `~/.aletheon/vector-db` to
   `${XDG_DATA_HOME:-$HOME/.local/share}/aletheon/vector-db`; do not keep both divergent copies.
3. Old `/tmp/agentd/overflow` and `/tmp/aletheon/aletheond.pid` are not trusted or auto-imported.
   After confirming no old daemon runs, they may be removed manually. Overflow content is disposable.
4. Legacy clients needing the machine socket can pass `--socket /run/aletheon/aletheon.sock`;
   normal users should omit it and use XDG runtime.
5. Rollback may continue reading the legacy LanceDB directory if it was not moved. If it was moved,
   explicitly configure `lance_path`; never copy a live database in both directions.

## Deterministic validation

```text
bash scripts/cargo-agent.sh test -p fabric types::paths::tests --lib
# 6 passed

bash scripts/cargo-agent.sh test -p executive host::tests --lib
# 3 passed

bash scripts/cargo-agent.sh test -p interact host::tests --lib
# 1 passed

bash scripts/cargo-agent.sh test -p mnemosyne impl::vector_store::tests --lib
# 6 passed

bash scripts/cargo-agent.sh test -p corpus tools::tools::output --lib
# 13 passed after the H8 collision/permission fixture

bash scripts/cargo-agent.sh check -p interact --tests
# passed
```

Final systemd user restart, cleanup and multi-instance exercises remain part of final SER8 validation.
