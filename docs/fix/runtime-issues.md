# Persistent Runtime Issues

Status: **Open** | Priority: High

---

## D1 — Daemon Runs as Nobody, Cannot Write User Project

- **Source:** aletheon-tui-observability memory (2026-07-06)
- **Severity:** High
- **Description:** The daemon drops privileges to `nobody` but then cannot write to the user's project directory. Falls back to `/tmp` for all file operations.
- **Impact:** File operations (code editing, file creation) target `/tmp` instead of the actual project. Session state is lost on reboot.
- **Fix direction:** Use user-level systemd unit (`systemctl --user`) to run as the invoking user; or implement a privileged helper that opens fd's before dropping privileges.

---

## D2 — bash_exec 10s Default Timeout Breaks cargo check

- **File:** `crates/corpus/src/tools/bash_exec.rs:48`
- **Source:** aletheon-tui-observability memory (2026-07-06)
- **Severity:** Medium
- **Description:** The default 10-second timeout for bash execution causes `cargo check` (and similar long-running commands) to always time out.
- **Impact:** Build/test commands cannot complete through the agent's bash tool.
- **Fix direction:** Increase default to 300s; add a per-command timeout override mechanism; stream partial output before timeout.

---

## D3 — /dev/null Denied Under Non-Root Sandbox

- **Source:** aletheon-tui-observability memory (2026-07-06)
- **Severity:** Medium
- **Description:** Under the non-root sandbox, access to `/dev/null` is denied. Many shell scripts and commands redirect to `/dev/null`.
- **Impact:** Commands that redirect output to `/dev/null` fail under sandbox.
- **Fix direction:** Add `/dev/null` to the sandbox allowlist for all workspace profiles.

---

## I2 — Sessions Never Persisted

- **Source:** aletheon-tui-observability memory (2026-07-06)
- **Severity:** High
- **Description:** `~/.aletheon/sessions/*` is always empty. The `/resume` command is a no-op shell. Session state does not survive daemon restart.
- **Impact:** No ability to resume conversations across restarts; violates the "persistent agent" design goal.
- **Fix direction:** Implement session serialization in the daemon shutdown path; load sessions on startup; implement `/resume` as a real command.

---

## I3 — audit session_id Always Empty

- **Source:** aletheon-tui-observability memory (2026-07-06)
- **Severity:** Low
- **Description:** The audit trail's `session_id` field is never populated, making it impossible to correlate audit events with sessions.
- **Impact:** Audit log is less useful for debugging and compliance.
- **Fix direction:** Thread `session_id` from the daemon's session manager into the audit event emission.
