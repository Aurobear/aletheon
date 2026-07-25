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

## Generalization and quality policy

- Never add production behavior keyed to an acceptance prompt, natural-language
  phrase, language, repository name, fixed checkout path, or expected test
  answer. Test fixtures may be specific; runtime policy must derive from typed
  state, capability semantics, effective configuration, and explicit budgets.
- Do not optimize a scenario by suppressing evidence collection. Path discovery
  is not content evidence, and fewer tools or model rounds are not improvements
  when the resulting answer is unsupported.
- Repository-analysis acceptance must verify that claimed paths and symbols
  exist and that absence claims are true. Architecture and maturity conclusions
  must be grounded in content actually returned to the model.
- Keep cumulative provider usage, active context occupancy, cache usage,
  inference rounds, retries, and tool calls as separate metrics. Never derive
  context-window percentage from cumulative billed/session tokens.
- Long-running acceptance requires multiple turns in one unchanged TUI session
  in addition to repeated fresh-session runs. A short follow-up must not pay for
  or display the entire cumulative token total as active context pressure.
- Any monitor PASS that disagrees with the rendered frame, persisted session,
  audit records, or daemon logs is a monitor defect and a failed acceptance.
