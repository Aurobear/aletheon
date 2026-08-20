# D6 external identity owner cutover

## Requirement receipt

- APX-01 requires Application to contain pure Rust types/use cases and eventually depend only on contracts/runtime.
- The boundary census assigned Fabric external identity rows to `Application identity/grant + external adapter translation` and physical deletion to D6 (historical rows 755-762 before this cutover).
- E2 keeps concrete Google/OAuth transport in the adapter.

## Cutover

- Provider-neutral identity/grant types moved to `crates/application/src/external_identity.rs`.
- Google source/event adapters consume Application identity vocabulary; OAuth and transport remain in Corpus Google modules.
- Executive repository/store and Aletheon wiring callers now import the Application owner types.
- Fabric module, root exports and source file were removed with no compatibility re-export.
- Existing UUID/serde validation, redaction and tests moved intact.
