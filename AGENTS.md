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
  `sudo bash scripts/aletheon.sh deploy` passes against the system-installed runtime. User-local deployment (`bash scripts/aletheon.sh deploy`) is a separate mode and must not be used for system acceptance.
- Final acceptance must prove that `target/release/aletheon`,
  `/usr/bin/aletheon`, and the executables of the running machine and user
  daemons have the same SHA-256 digest.
- Final acceptance must observe stable systemd restart counters and complete a
  real LLM request using `/usr/bin/aletheon` and the official user socket.
- Test-only profiles and assets must remain under an isolated home/state root.
  Do not copy them into `~/.local/state/aletheon` unless the matching release
  binary is installed in the same deployment.

## Debugging evidence policy

- Repository-overview tests must batch-read known entry files before scoped discovery; extension-by-extension inventories are a regression.
- Report model inference rounds, provider retries, and tool calls separately. A lower tool count does not prove fewer provider requests.
- `provider_unavailable`, `provider_rejected_request`, or any rendered inference error makes a real-TUI run fail even if the prompt returns or a monitor aggregate reports PASS.
- Validate monitor verdicts against the rendered frame, session evidence, and daemon logs.
- Model-controlled routing or arguments require three consecutive real-TUI runs; deterministic unit and contract tests do not.
