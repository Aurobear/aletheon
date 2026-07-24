# Repository Agent Instructions

## Rust build resource policy

- Do not invoke `cargo` directly for repository builds, checks, tests, lint, or docs.
- Run Cargo through `bash scripts/cargo-agent.sh <cargo arguments>` so concurrent
  worktrees share one bounded build cache and one global compilation lock.
- Use the narrowest package and test target that validates the change.
- Only the integration/verification owner may run workspace-wide checks.
- Do not run concurrent `executive` or workspace builds.
- Formatting may use `bash scripts/cargo-agent.sh fmt --all -- --check`.

## Installed runtime acceptance policy

- Development binaries, temporary homes, alternative sockets, isolated daemons,
  direct provider calls, and direct bridge tests are diagnostic evidence only.
  They must never be reported as final deployment acceptance.
- Changes affecting tools, agent profiles, configuration, persistence, IPC,
  daemon bootstrap, or client behavior are complete only after
  `bash scripts/aletheon.sh deploy` passes against the installed runtime.
- Final acceptance must prove that `target/release/aletheon`,
  `/usr/bin/aletheon`, and the executables of the running machine and user
  daemons have the same SHA-256 digest.
- Final acceptance must observe stable systemd restart counters and complete a
  real LLM request using `/usr/bin/aletheon` and the official user socket.
- Test-only profiles and assets must remain under an isolated home/state root.
  Do not copy them into `~/.local/state/aletheon` unless the matching release
  binary is installed in the same deployment.
