# Production hardening H0 baseline — 2026-07-22

## Scope

This record satisfies H0 in
`docs/plans/2026-07-21-production-readiness-hardening.md:83-97`. It captures the
current dependency, configuration and operational entry points before H1 code
changes. It is evidence for the current working tree, not a release acceptance.

## Architecture facts

| Claim | Current evidence |
|---|---|
| Corpus production dependencies are Fabric, Kernel and Platform | `crates/corpus/Cargo.toml:9-12` |
| Execd depends on Platform, not Corpus | `crates/execd/Cargo.toml:8-16` |
| Canonical MCP configuration is owned by Corpus | `crates/corpus/src/tools/mcp/config.rs:11-39` |
| Executive is the broad composition root | `crates/executive/Cargo.toml:9-19` |
| Interact production code depends on Fabric; Executive is test-only | `crates/interact/Cargo.toml:9-30` |
| Cognit production code depends on Fabric; Kernel is test-only | `crates/cognit/Cargo.toml:9-31` |
| Executive's Hardware edge is test-only | `crates/executive/Cargo.toml:9-19,47-48` |

The human snapshot is
`docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md`; the controlled
machine snapshot is `config/architecture-dependencies.txt`, generated and
compared by `scripts/architecture-check.sh:582-668`. Test-only edges explicitly
reviewed in that script are not reported as production coupling.

## Operational entry points

```text
scripts/aletheon.sh configure check
  -> validates configuration paths and endpoint syntax
scripts/aletheon.sh health
  -> core socket + user daemon health RPC + configured GBrain health
scripts/aletheon.sh verify
  -> configuration + installed service/timer/runtime verification
```

The daemon RPC request and readiness mapping are implemented in
`scripts/aletheon-healthcheck.sh:69-98,148-164`. The dispatcher exposes the
operator commands at `scripts/aletheon.sh:18-64`; their implementation is in
`scripts/lib/aletheon/verify.sh:3-31`.

## Reproducible verification

Run from the repository root on the SER8 host:

```bash
bash scripts/architecture-check.sh
bash scripts/aletheon.sh configure show
bash scripts/aletheon.sh configure check
bash scripts/aletheon.sh health
bash scripts/aletheon.sh verify
```

Result on 2026-07-22:

- architecture gate: PASS (`28 findings, 36 dependencies, 4 paths; no additions`);
- configuration syntax and paths: PASS;
- core socket readiness: `ready`;
- user daemon liveness/readiness: `alive` / `ready`;
- GBrain health: PASS (`status=ok`, PGlite engine);
- deployed-state verification: PASS, including Pi coder and Pi RPC registration evidence.

No service or computer restart was required for H0.

## H0 disposition

- Old Corpus, Execd and MCP ownership claims were corrected in the current
  architecture snapshot.
- The architecture dependency baseline and reviewed test-only exceptions now
  agree with current Cargo metadata.
- Current Markdown references do not depend on the deleted plan files.
- The canonical operator path can reproduce the deployed health baseline.

H0 is complete. H1 may start with external-input panic candidate discovery and
must still obey its reproduce-before-edit stopping condition.
