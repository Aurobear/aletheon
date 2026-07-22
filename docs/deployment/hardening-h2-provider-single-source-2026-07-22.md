# Production hardening H2 Provider single-source evidence — 2026-07-22

## Requirement

H2 is defined at
`docs/plans/2026-07-21-production-readiness-hardening.md:129-150`: one canonical
provider definition, one production construction implementation, explicit
transport authority, tested `Auto` compatibility, and one interpretation of
runtime parameters and credential identity.

## Previous behavior split

Two unrelated structs were both named `ProviderConfig`:

- application transport/configuration at `crates/cognit/src/config/mod.rs`;
- heuristic local/cloud routing metadata at
  `crates/cognit/src/impl/inference/provider_config.rs`.

Provider construction was independently implemented in
`impl/provider_registry.rs` and `impl/llm/provider_factory.rs`. The scheduler
then converted canonical configuration into a third field set containing a
free-form `kind`, and credential environment names were assembled in three
places. Consequences included different Ollama handling and missing scheduler
timeout/context/token propagation.

## Implemented boundary

```text
config::ProviderConfig                 canonical deployment definition
        |
        +-> resolve_provider_definition
        |     explicit Transport wins
        |     Auto -> tested compatibility heuristic
        |     credential identity + context + pricing
        |
        +-> provider_factory::create_provider
              ProviderBuildOptions(timeout + max_tokens)
              OpenAI / Anthropic / Ollama
              ^                         ^
              |                         |
       ProviderRegistry          LlmScheduler
```

Changes:

- `Transport` now supports explicit `ollama`; `Auto` remains the default only
  for backward compatibility;
- `provider_factory::create_provider` is the only concrete constructor owner;
- Registry and scheduler delegate to the same factory and pass the same
  timeout/max-token options;
- scheduler provider entries wrap the canonical `ProviderConfig` instead of
  repeating name/base URL/API key/kind fields;
- the heuristic router type is renamed `InferenceCandidate` with
  `ProviderClass`, making clear that it is model-selection metadata rather than
  deployment configuration;
- API-key fallback identity is assembled only in the factory. H3 will replace
  raw business-environment discovery with typed secret references/preflight;
- the checked-in JSON schema includes explicit `ollama`;
- `scripts/architecture-check.sh` enforces exactly one Cognit
  `ProviderConfig` definition and rejects concrete constructors outside the
  canonical factory.

## Acceptance evidence

Static ownership checks:

```text
rg 'struct ProviderConfig' crates/cognit/src        -> exactly one
rg 'AnthropicProvider::new|OpenAiProvider::new|OllamaProvider::new' \
  crates/cognit/src                                 -> provider_factory.rs only
rg 'create_provider_by_kind|detect_provider_kind|detect_transport' \
  crates/cognit/src                                 -> no matches
```

Tests:

```bash
bash scripts/cargo-agent.sh test -p cognit provider_factory
bash scripts/cargo-agent.sh test -p cognit provider_registry
bash scripts/cargo-agent.sh test -p cognit scheduler::tests
bash scripts/cargo-agent.sh test -p cognit inference::router::tests
bash scripts/cargo-agent.sh test -p cognit
bash scripts/cargo-agent.sh test -p executive --test deterministic_snapshots \
  app_config_schema_matches_checked_in_repository_snapshot -- --exact
bash scripts/cargo-agent.sh test -p executive --test layered_config_contract
bash scripts/architecture-check.sh
bash scripts/cargo-agent.sh fmt --all -- --check
git diff --check
```

Results:

- canonical factory: 4/4 PASS, covering explicit authority, Auto compatibility,
  credential identity, pricing/context metadata and all three protocols;
- Registry: 6/6 PASS;
- Scheduler: 8/8 PASS;
- heuristic inference router: 9/9 PASS;
- full Cognit package: 303 tests PASS (287 unit + 16 integration);
- schema snapshot: PASS;
- layered configuration contract: 7/7 PASS;
- architecture gate and formatting: PASS.

Existing non-fatal warnings in `cognit/tests/facade_contract.rs` remain outside
this batch; no unrelated lint cleanup was included.

## Disposition

H2 is complete. Provider definition and creation now have one production owner,
and the architecture gate prevents the former split from returning. H3 may
replace direct business environment discovery with typed configuration,
credential references and enabled-integration preflight.
