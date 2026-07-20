# Repository Agent Instructions

## Rust build resource policy

- Do not invoke `cargo` directly for repository builds, checks, tests, lint, or docs.
- Run Cargo through `bash scripts/cargo-agent.sh <cargo arguments>` so concurrent
  worktrees share one bounded build cache and one global compilation lock.
- Use the narrowest package and test target that validates the change.
- Only the integration/verification owner may run workspace-wide checks.
- Do not run concurrent `executive` or workspace builds.
- Formatting may use `bash scripts/cargo-agent.sh fmt --all -- --check`.

