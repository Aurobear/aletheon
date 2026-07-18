# Flaky Tests

Status: **Known, Not a Regression** | Priority: Low

---

## corpus::hook::registry::tests::execute_script_hook_inject

- **File:** `crates/corpus/src/hook/registry.rs:269`
- **Source:** aletheon-flaky-script-hook-test memory
- **Severity:** Low
- **Description:** This test flakes (fails non-deterministically) under `cargo test --workspace` with parallel execution. Passes reliably when run in isolation.
- **Impact:** Does not affect CI — CI runs `cargo test` per-crate, which rarely triggers the race condition.
- **Root cause:** Likely a shared resource or ordering dependency between test cases in different crates.
- **Fix direction:** Investigate shared test state (temp dirs, env vars, global state); add test isolation.

---

## tui_checks False Positives on User Input Lines

- **Source:** aletheon-tui-observability memory (2026-07-06)
- **Severity:** Low (test harness limitation)
- **Description:** The TUI test harness (`tui_checks`) false-positives on echoed user input lines containing `|` table syntax or `/path` prefixes, misinterpreting them as TUI rendering artifacts.
- **Impact:** Flaky TUI test results; not a product bug.
- **Fix direction:** Improve the TUI output parser to distinguish user input echo from TUI rendering.
