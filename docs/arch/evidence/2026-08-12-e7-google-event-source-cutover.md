# E7 Google external source/event Fabric cutover evidence

## Requirement receipt

- E2 keeps Gmail/Google transport and concrete adapter behavior outside core owners (`docs/plans/2026-08-08-preserved-extensions-cutover.md:379-384`).
- APX-03 keeps Application's use case provider-neutral while Gmail/Google names remain in the extension adapter (`docs/plans/2026-08-08-application-persistence-extraction.md:236-245`).
- E7 removes only extension-owned Fabric rich types after cutover and leaves unrelated shared rows for D6 (`docs/plans/2026-08-08-preserved-extensions-cutover.md:418-426`).

## Cutover

- Source DTO authority moved from `crates/fabric/src/types/external_source.rs` to `crates/corpus/src/tools/google/source.rs`.
- Normalized Google event authority moved from `crates/fabric/src/types/external_event.rs` to `crates/corpus/src/tools/google/event.rs`.
- Application's provider-neutral `ExternalStimulus` remains at `crates/application/src/goal_draft.rs:13-42`; the rich Google envelope was not copied into Application.
- Gateway's provider-specific event registry was removed. The concrete capability seam is now `GoogleEventCapabilityHandler` beside the Google event router in `crates/executive/src/adapters/google/event_dispatcher.rs`.
- Fabric module declarations, root exports, source/event files, and all production/test callers were removed. No compatibility re-export was added.
- Persisted v1 read/v2 write compatibility moved intact with the event owner; the Google event store remains the single persistence writer.

## Static gates

```text
rg 'types::external_(event|source)|fabric::external_(event|source)|fabric::ExternalEvent|fabric::ExternalRecordRef' crates --glob '*.rs'
# zero

rg 'EventCapabilityHandler|EventCapabilityRegistry' crates/gateway --glob '*.rs'
# zero
```

Validation and installed-runtime acceptance are recorded after the focused and closeout checks complete.

## Validation and installed-runtime acceptance

- `bash scripts/cargo-agent.sh test -p corpus --lib tools::google`: 22 passed.
- Executive Gmail ingress/routing/recovery focused suite: 9 passed.
- Changed validation report `/tmp/aletheon-changed-validation.json`: 104/104 passed.
- L2: diff check, formatting, workspace/all-target check, architecture acceptance and script-surface gate passed.
- `sudo bash scripts/aletheon.sh deploy`: passed.
- Release, installed binary, machine daemon and user daemon SHA-256 all equal:
  `be94fc84f55ffcd114ba46a5550b3df5a94ae4e35548e8f40e37df267ec0b574`.
- Both services remained `active`, `ExecMainStatus=0`, `NRestarts=0` across the stability interval.
- Official Memory Agent protocol smoke and official-client real-request smoke passed.
- Installed package preservation observed one package with one agent profile, one connector, one hook and four skills.
- No L3 veto signature was present in the deployment journal window.
