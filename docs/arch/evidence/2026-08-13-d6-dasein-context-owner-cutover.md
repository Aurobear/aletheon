# D6 Dasein context owner cutover evidence

Date: 2026-08-13

## Requirement receipt

- D6 closes non-extension rich surfaces before Executive retirement: .
- Rich domain models must move to their domain owner rather than remain in the dependency-neutral contract root: .

## Cutover

- Dasein now owns its context/read-model snapshots in `crates/dasein/src/dasein/context.rs`.
- `DaseinOps`, Dasein state implementations, prefix composition, Metacog and Executive diagnostics consume the Dasein-owned models.
- `crates/contracts/src/dasein/context.rs` and its contract-root reexports are deleted.
- The D0 inventories no longer list the deleted context models.

The remaining `contracts::dasein` types are transition/wire types currently consumed by Runtime, Gateway protocol, Agora, and neutral conscious-workspace DTOs. They are deliberately not moved in this slice because doing so would create a Runtime-to-Dasein dependency cycle before their wire boundary is split.

## Focused validation

```text
bash scripts/cargo-agent.sh check -p dasein --all-targets PASS
bash scripts/cargo-agent.sh check -p cognit --all-targets PASS
bash scripts/cargo-agent.sh check -p metacog --all-targets PASS
bash scripts/cargo-agent.sh check -p executive --all-targets PASS
bash scripts/cargo-agent.sh check -p aletheon --all-targets PASS
```
