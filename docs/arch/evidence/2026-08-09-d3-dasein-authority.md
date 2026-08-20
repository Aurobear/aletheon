# Agent Kernel V2：D3 Dasein/Metacog authority convergence

Date: 2026-08-09
Baseline: `0bf690b2`
State: Dasein authority seam established; no writer cutover, no facade deletion

## Context receipt (runbook §3.1)

```text
Slice: D3 Dasein/Metacog authority convergence (port seam)
Baseline commit: 0bf690b2
Direct prerequisites: D1 (contracts) + D2 (Cognit split) — done
Current authoritative writer: unchanged legacy Dasein self-mutation / Executive Self/Metacog facade
Target owner/writer: Dasein (SelfMutationAuthority); Runtime durable outbox feeds consumers at cutover
IDs minted here: none
Production callers: none yet (ports additive)
Test-only callers: 1 authority test
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: Executive Self/Metacog facade → XRET-02
Unknowns/blockers: none for the seam; rich-type return + adapter moves are the cutover
Expected files: crates/dasein/src/core/ports.rs, core/mod.rs, Cargo.toml, D3 gate
Out-of-scope files: rich-type Fabric→owner return, repository/sandbox/evaluator adapter moves, facade deletion
```

## 1. What was created

`crates/dasein/src/core/ports.rs`:

- **`SelfMutation`** — Dasein-owned self-mutation request (version + mutation).
- **`SelfMutationAuthority`** — the single authority that commits Self mutation; Metacog observes/proposes, never mutates.
- **`PostSettlementConsumer`** — a consumer fed by the Runtime durable outbox (two post-settlement consumers connect at the cutover).
- **`DaseinError`** — typed MutationRejected / ConsumerUnavailable.

## 2. Rules honoured (D3)

- Dasein is the **only** Self-mutation authority (Metacog observes/evaluates/proposes/experiments — never mutates): `SelfMutationAuthority::commit` is the only commit path.
- Rich types from Fabric return to owner at the cutover (documented; not moved here).
- repository/sandbox/coding-evaluator move to adapters at the cutover.
- Runtime durable outbox feeds two post-settlement consumers (`PostSettlementConsumer` port defined).
- Executive Self/Metacog facade deletion is D3 PR-C / XRET-02 — not this seam.
- D3 gate rejects any `Metacog`/`Executive` leak into the Dasein authority production code.

## 3. D3 gate (architecture-check.sh)

`ARCH_SKIP_D3_GATES` rejects any `Metacog*`/`Executive` reference in the Dasein authority production code; requires `SelfMutationAuthority` + `PostSettlementConsumer`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p dasein        PASS
bash scripts/cargo-agent.sh test -p dasein --lib core::ports  PASS (1 passed)
  - authority_commits_self_mutation
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `ports.rs` + core/mod.rs line + Cargo.toml thiserror + D3 gate → exact baseline. No schema, no writer, no facade change.

## 6. Next

D4 (Agora/Mnemosyne convergence) and D5 (Corpus catalog/executor split) follow; then D6 non-extension Fabric cleanup.
