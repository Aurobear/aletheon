# E7 unused extension state retirement — 2026-08-12

E7 removes extension-specific Fabric rich types after caller-zero
(`docs/plans/2026-08-08-preserved-extensions-cutover.md:418-428`). The census
records `ActivationState`, `HealthState`, and `ActivationTransition` as having no
production caller and assigns them to Application extension activation with E7
deletion (`config/architecture/fabric-boundary-census.tsv:743-745`).

Repository-wide source inspection found no consumer beyond the defining module
and two Fabric-only serde characterization tests. No Application activation
implementation uses these types; retaining them would preserve an ownerless
second state vocabulary. The module, declaration, and obsolete tests are
therefore deleted rather than copied.

```bash
! rg -n 'fabric::types::extension_state|\bActivationTransition\b' crates --glob '*.rs'
! test -e crates/fabric/src/types/extension_state.rs
```
