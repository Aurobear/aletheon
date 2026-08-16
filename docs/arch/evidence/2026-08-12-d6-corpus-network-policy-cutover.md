# D6 Corpus network policy cutover evidence

- Date: 2026-08-12
- Scope: move the outbound tool-execution network policy from the shared Fabric contract root to its Corpus capability-execution owner.
- Source removed: `crates/corpus/src/security/network_policy.rs`.
- Canonical owner: `crates/corpus/src/security/network_policy.rs`.
- Production/configuration callers now consume `corpus::security::network_policy` directly.
- Compatibility: no Fabric re-export remains; the two retired Fabric census rows were removed and the frozen public baseline was reduced from 780 to 778.
- Validation: focused Corpus, aletheon-config, Aletheon, Executive, and Fabric checks plus the architecture suite.
