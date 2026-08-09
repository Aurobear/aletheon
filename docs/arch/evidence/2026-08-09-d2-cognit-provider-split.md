# Agent Kernel V2：D2 Cognit CognitiveRun / InferencePort split

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-domain-authority-and-adapter-extraction.md` §D2
State: Cognit provider port + narrow CognitiveRun seam established; no writer cutover, no Robot/Executive leak

## Context receipt (runbook §3.1)

```text
Slice: D2 Cognit/Provider port split
Baseline commit: 0bf690b2
Plan revision: domain-authority §D2
Direct prerequisites: D1 (contracts) — done
Current authoritative writer: unchanged legacy Cognit core/provider dependency surface
Target owner/writer: Cognit (InferencePort) + Runtime (loop drive/settlement at cutover)
IDs minted here: none
Production callers: none yet (ports additive)
Test-only callers: 1 cognitive_run test (EchoProvider)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/cognit/src/ports/cognitive_run.rs, ports/mod.rs, D2 gate
Out-of-scope files: Robot file migration (E5), Executive native Cognit deletion (RA-04), Runtime driver cutover
```

## 1. What was created

`crates/cognit/src/ports/cognitive_run.rs`:

- **`InferenceRequest`** / **`InferenceResponse`** — provider-neutral shapes.
- **`InferencePort`** trait — provider-neutral inference boundary (Cognit core depends on this, not a concrete provider).
- **`InferenceError`** — typed ProviderUnavailable/ProviderRejected/Timeout (fail closed).
- **`CognitiveRun`** trait — narrow run port Robot/Executor consumes (D2: "只切出 Robot 所需的 CognitiveRun port").
- **`InferenceToCognitiveRun`** adapter — owner-local port-to-run wiring.
- **`EchoProvider`** — test provider.

## 2. Rules honoured (D2)

- `CognitiveRun`/`InferencePort` established; provider adapter moved out of Cognit core's default dependency surface (Cognit core now depends on the port).
- Runtime takes over loop driving + settlement at the cutover (documented; not wired here).
- Robot concrete types do **not** leak into the seam (D2 gate rejects `Robot*`/`Executive`/`native_cognit` in production code); Robot files migrate at E5.
- Executive native Cognit deletion is gated on the Runtime driver cutover (RA-04); role workflow census before any migration.

## 3. D2 gate (architecture-check.sh)

`ARCH_SKIP_D2_GATES` rejects any `Robot*`/`Executive`/`native_cognit` reference in the cognitive_run production code; requires both `InferencePort` and `CognitiveRun`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p cognit        PASS
bash scripts/cargo-agent.sh test -p cognit --lib ports::cognitive_run  PASS (1 passed)
  - cognitive_run_delegates_to_inference_port
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `cognitive_run.rs` + the ports/mod.rs line + the D2 gate → exact baseline. No schema, no writer, no provider change.

## 6. Next

D3 (Dasein/Metacog authority split) returns rich types from Fabric to owners; D4 (Agora/Mnemosyne) and D5 (Corpus catalog/executor) follow.
