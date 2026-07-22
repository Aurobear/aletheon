# Phase 4 Configuration Ownership Implementation Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Separate deployment parsing, normalized configuration, domain policy, and adapter construction so Cognit and other domains no longer own unrelated channel or coding adapter settings.

**Architecture:** Keep raw compatibility aliases in Executive composition, normalize once, pass validated domain configs to domains and adapter configs/credential handles only to adapter factories.

**Tech Stack:** Rust 1.85+, Bash, Python 3, Cargo via `scripts/cargo-agent.sh`, repository architecture gates.

---

## Global execution constraints

- Treat `docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md` as the architecture source of truth.
- Re-read that document and every cited symbol before editing; record changed line anchors in the task report.
- Do not modify files outside the declared paths. Stop if a required change crosses the boundary and report it.
- Preserve unrelated working-tree changes. Never use `git reset --hard`, `git checkout --`, or broad cleanup commands.
- Never invoke Cargo directly. Use `bash scripts/cargo-agent.sh <cargo arguments>` and the narrowest package/test target.
- Do not run concurrent Executive or workspace builds. Only the final integration owner runs workspace-wide commands.
- Keep security-sensitive behavior fail-closed. Do not weaken credential, scope, sandbox, network, lease, or trust checks.
- Each non-trivial commit must use a conventional subject, blank line, problem/solution context, and concrete bullets.
- Before each commit run `git diff --cached --check` and inspect the complete staged diff.
- A task is incomplete if tests pass but its architecture gate, compatibility evidence, or inventory update is missing.

## Prerequisites and owned paths

Prerequisite: Phase 2; coordinate canonical coding config with Phase 3.

- Modify: `crates/executive/src/core/config/` or Phase 2 composition config path
- Modify: `crates/executive/src/user_runtime/`
- Modify: `crates/cognit/src/config/mod.rs`
- Modify: Gateway/channel and coding adapter config owners
- Modify: checked-in config schema and layered-config tests

## Task 1: Inventory every config field and owner

For every AppConfig field record:

```text
raw key | compatibility aliases | normalized owner | consumer | secret? | default | validation
```

- [ ] Identify Telegram/channel fields currently owned by Cognit.
- [ ] Identify Pi/coding runtime fields currently owned by Cognit.
- [ ] Identify GBrain/supplemental fields for Phase 5.
- [ ] Identify provider/inference fields for Phase 7.
- [ ] Identify deployment paths/env reads mixed into domain configs.

## Task 2: Define the one-way config pipeline

Implement/facade:

```text
DeploymentConfig -> NormalizedConfig -> DomainConfig + AdapterBuildConfig
```

- [ ] Only composition reads files and business environment variables.
- [ ] Compatibility aliases exist only in DeploymentConfig decoding.
- [ ] NormalizedConfig has canonical names and validated URLs/IDs/bounds.
- [ ] DomainConfig contains policy only, no SecretRef, endpoint, env name, or deployment path.
- [ ] AdapterBuildConfig may contain SecretRef; resolved plaintext never returns to config.

## Task 3: Move channel and coding settings

- [ ] Move TelegramConfig to Gateway adapter/composition ownership.
- [ ] Move Pi/coding configuration to Executive coding adapter ownership.
- [ ] Preserve old TOML/env inputs through deterministic compatibility conversion.
- [ ] Add old-config -> canonical-config snapshot tests.
- [ ] Remove channel/coding types from Cognit public config API.

## Task 4: Static adapter registry

- [ ] Define validated AdapterId and IntegrationKind.
- [ ] Permit adapter-ID constructor matching only inside composition registry/factory.
- [ ] Reject unknown adapter IDs explicitly.
- [ ] Remove URL suffix/provider inference.
- [ ] Do not introduce a dynamic plugin ABI.

## Task 5: Schema and diagnostics

- [ ] Regenerate `config/schema/aletheon-config.schema.json` through the repository-supported command/test.
- [ ] Verify unknown fields fail and compatibility aliases normalize.
- [ ] Verify Debug/log output redacts resolved values.
- [ ] Ensure invalid configured optional integrations fail closed rather than silently disappear.

## Validation

```bash
bash scripts/cargo-agent.sh test -p executive --test layered_config_contract
bash scripts/cargo-agent.sh test -p executive --test private_composition_root
bash scripts/cargo-agent.sh test -p cognit config
bash scripts/cargo-agent.sh test -p gateway
bash tests/architecture_check.sh
```

## Commit stages

1. `test(config): lock layered and legacy normalization behavior`
2. `refactor(config): separate deployment and normalized configuration`
3. `refactor(config): return channel and coding settings to adapter owners`
4. `refactor(composition): centralize static adapter construction`
5. `chore(config): regenerate schema and enforce ownership gates`
