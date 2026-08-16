# XRET-05 unused legacy checkpoint retirement — 2026-08-13

The legacy in-memory `executive::core::checkpoint` implementation had no caller
outside its own module. Production workspace checkpoint behavior is provided by
the typed `application::workspace_checkpoint` service and its injected adapter.
The dead module, module declaration, and governed Executive inventory rows were
therefore removed rather than migrated.

Evidence:

- Exact caller search returned only self-definitions before deletion.
- `bash scripts/cargo-agent.sh check -p executive --lib`: passed.
- `ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture`:
  passed with no additions.
- formatting and `git diff --check`: passed.
