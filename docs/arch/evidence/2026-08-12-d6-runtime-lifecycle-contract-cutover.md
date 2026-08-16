# D6 Runtime lifecycle contract cutover evidence

- Date: 2026-08-12
- Canonical owner: `crates/runtime/src/lifecycle.rs`.
- Executive lifecycle dispatch and effect validation consume Runtime directly.
- Fabric module and four census/public rows were removed; baseline changed from 759 to 755.
