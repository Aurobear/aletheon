# D6 Application data-governance cutover evidence

- Date: 2026-08-12
- Canonical owner: `crates/application/src/data_governance.rs`.
- Cognit, Mnemosyne, and Corpus projection boundaries now consume the Application-owned trust/classification and scrub policy directly.
- Fabric module/export and three census rows were removed; public baseline changed from 769 to 766.
