# D6 Dasein self-field owner cutover evidence

Date: 2026-08-12

## Requirement receipt

- D6 requires rich domain facades to leave the dependency-neutral contracts crate: .
- The deletion sequence places D6 before the final Executive retirement: .

## Cutover

- Dasein now owns the self-field domain contract in `crates/dasein/src/core/contracts.rs`.
- Dasein's implementation and tests consume that owner directly.
- Cognit, Mnemosyne, Metacog, and Executive consume the Dasein-owned public types directly.
- The retired `crates/contracts/src/include/self_field.rs` surface and its root re-exports are absent.
- The boundary and public-type inventories no longer claim the deleted shared surface.

## Dependency-cycle prevention

Cognit-owned evolution inputs formerly consumed by Dasein were narrowed into the
Dasein-owned `EvolutionTrigger`, `LearnedRuleSnapshot`, and `BehaviorAdjustment`
input shapes. Cognit maps its richer learning records at the adapter boundary;
Dasein does not depend on Cognit.

## Focused verification

```text
bash scripts/cargo-agent.sh check -p dasein --all-targets   PASS
bash scripts/cargo-agent.sh check -p cognit --all-targets  PASS
bash scripts/cargo-agent.sh check -p metacog --all-targets PASS
bash scripts/cargo-agent.sh check -p executive --all-targets PASS
```

Installed-runtime acceptance is intentionally deferred until all production
migration slices pass the deterministic closeout gate.
