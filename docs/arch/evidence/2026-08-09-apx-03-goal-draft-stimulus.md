# Agent Kernel V2：APX-03 Goal Draft + External Stimulus provider-neutral use cases

Date: 2026-08-09
Baseline: `0bf690b2`
State: provider-neutral Goal Draft + External Stimulus use cases established; no cutover

## Context receipt (runbook §3.1)

```text
Slice: APX-03 Goal Draft + External Stimulus provider-neutral use cases
Baseline commit: 0bf690b2
Direct prerequisites: APX-02 (Approval) — done
Current authoritative writer: unchanged legacy goal service / worker (Gmail/Telegram goal commands)
Target owner/writer: Application GoalDraft (provider-neutral); not yet wired
IDs minted here: none
Production callers: none yet (use cases additive)
Test-only callers: 2 goal_draft tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/application/src/goal_draft.rs, lib.rs re-export, APX-03 gate
Out-of-scope files: Gmail/Google adapter, goal worker/budget/verification, cutover
```

## 1. What was created

`crates/application/src/goal_draft.rs`:

- **`ExternalStimulus`** — provider-neutral stimulus (`channel` opaque label, `content`, `correlation`).
- **`GoalDraft`** — provider-neutral goal draft.
- **`create_goal_draft`** — pure constructor.
- **`ingest_external_stimulus`** — maps any stimulus to a draft; works with the channel **disabled**.

## 2. Rules honoured (APX-03)

- `CreateGoalDraft` / `IngestExternalStimulus` provider-neutral: **no Gmail/Google/OAuth/transport names anywhere in Application code** (APX-03 gate rejects them in production; the metric `CORE_EXTERNAL_IDENTIFIER_HITS` stays at baseline).
- Channel OAuth/transport stay in the extension adapter (E2-K6a owns them).
- Goal attempt/worker/budget/verification **not copied** into Application (gate rejects `AttemptWorker`/`GoalWorker`/`BudgetLedger`/`Verifier`).
- Shared `objectives.db` per-owner bundles at the cutover (not in this seam).
- Acceptance: with channel disabled, generic stimulus still ingests (test `generic_stimulus_ingests_without_gmail`).

## 3. APX-03 gate (architecture-check.sh)

`ARCH_SKIP_APX03_GATES` rejects any provider name (`Gmail`/`Google`/`OAuth`/`oauth`/`Transport`/`send`) and any copied worker/budget/verifier symbol in Application goal-draft production code (comments + tests exempt).

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p application    PASS
bash scripts/cargo-agent.sh test -p application --lib goal_draft  PASS (2 passed)
  - generic_stimulus_ingests_without_gmail
  - goal_draft_is_provider_neutral
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps, metric baseline stable)
```

## 5. Remaining APX-03 (cutover)

The goal-service/Gmail-route switch to the provider-neutral use cases, per-owner `objectives.db` bundle management, and Gmail preservation smoke are the cutover (E2-K6a co-owns the Gmail side). Rollback keeps the old facade one-way; sent messages are never replayed.

## 6. Rollback

Delete `goal_draft.rs` + lib.rs re-export + APX-03 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

APX-04 (filesystem/SQLite/process/host adapters out of Application core) removes concrete-I/O imports from the Application layer.
