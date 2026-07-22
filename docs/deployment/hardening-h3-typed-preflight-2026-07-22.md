# H3 typed config and startup preflight evidence — 2026-07-22

## Requirement anchors

- Enabled integrations must fail with a typed diagnostic before core startup when required
  configuration is missing (`docs/plans/2026-07-21-production-readiness-hardening.md:169-174`).
- Disabled integrations and OAuth client types that do not need a secret must not be rejected
  (`docs/plans/2026-07-21-production-readiness-hardening.md:163-167`).
- Business environment parsing belongs to host/bootstrap; domain services receive typed config,
  `SecretRef`, or a capability (`docs/plans/2026-07-21-aletheon-coupling-and-external-interface-audit.md:205-220`).
- Legacy variables require explicit precedence, deprecation, and tests
  (`docs/plans/2026-07-21-production-readiness-hardening.md:171-174`).

## Implemented contract

```text
system/user/project config
          |
legacy business env --(deprecated alias; native ALETHEON__ wins)--+
          |                                                    |
          +------> Executive typed AppConfig + provenance <----+
                                  |
                         startup preflight
                     +------------+------------+
                     |                         |
               disabled: None            enabled: validate
                                               |
                                    CredentialResolver(SecretRef)
                                               |
                            ResolvedIntegrations (redacted Debug)
                                  /                    \
                         Google adapter          search adapter
```

- `SecretRef`, redacted `SecretValue`, `CredentialResolver`, OAuth client type, typed bootstrap
  values, and Google/search integration inputs are defined at
  `crates/executive/src/core/config/integrations.rs:10-208`.
- Google enablement continues to use the existing authoritative
  `deployment.integrations.google` flag; its public OAuth mode does not require a client secret
  (`crates/executive/src/core/config/mod.rs:109-119`,
  `crates/executive/src/core/config/integrations.rs:157-188`).
- Runtime bootstrap performs integration preflight before provider, storage, session, or worker
  construction (`crates/executive/src/core/runtime_core.rs:56-72`).
- Google bootstrap consumes only resolved typed values
  (`crates/executive/src/impl/daemon/bootstrap/google.rs:29-68`).
- `WebSearchTool` consumes host-injected configuration and no longer reads business environment
  variables during execution (`crates/corpus/src/tools/tools/web_search.rs:10-53,100-150`).
- Legacy runtime, Google, and search variables are translated only in Executive's config loader;
  native `ALETHEON__` values win, secret values are converted to references rather than copied,
  and use is logged as deprecated (`crates/executive/src/core/config/mod.rs:321-457`).
- The H3 architecture gate rejects reintroduction of those direct business-environment reads
  outside Executive config (`scripts/architecture-check.sh:94-105`). Host protocol variables such
  as systemd, XDG/display, credential directory, sockets, and subprocess handoff remain in scope.

## Deterministic verification

Run through the repository build wrapper only:

```bash
bash scripts/cargo-agent.sh test -p executive core::config --lib
bash scripts/cargo-agent.sh test -p executive --test layered_config_contract
bash scripts/cargo-agent.sh test -p corpus web_search --lib
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/architecture-check.sh
git diff --check
```

Covered cases:

- disabled integrations require no fields or credentials;
- public Google OAuth does not require `client_secret`;
- confidential Google OAuth requires a secret reference;
- enabled search reports the exact missing typed path;
- preflight diagnostics include source kind but redact source locator and secret values;
- native typed environment overrides a legacy alias;
- legacy secret values never enter the typed configuration tree;
- legacy Drive CSV becomes a typed array;
- web search disabled and network-policy rejection paths are deterministic;
- checked-in JSON Schema matches the generated `AppConfig` schema.

## Operational compatibility and rollback

- Legacy variables remain accepted for this compatibility window. Precedence is:
  explicit config/CLI layer > native `ALETHEON__...` > translated legacy variable > file/default
  layers. Native typed environment always wins when both environment forms exist.
- Legacy variable names are logged, but values are never logged.
- Rollback is the independent H3 commit. It does not change a database schema or persistent data.
