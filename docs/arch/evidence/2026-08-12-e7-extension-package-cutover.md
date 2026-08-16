# E7 extension package and asset ownership cutover — 2026-08-12

## Anchors

E7 deletes extension-owned Fabric rich types after authoritative cutover and
caller-zero (`docs/plans/2026-08-08-preserved-extensions-cutover.md:418-428`).
Package rows target the Application extension package/Corpus catalog and E7
retirement (`config/architecture/fabric-boundary-census.tsv:736-742`); adjacent
asset rows target the Corpus catalog (`config/architecture/fabric-boundary-census.tsv:728-735`).

## Result

Package DTOs now live in `crates/corpus/src/extension/package.rs`; asset,
runtime-class and capability descriptor DTOs live in
`crates/corpus/src/extension/asset.rs`. Corpus package parsing/store/resolution,
Aletheon production composition, `aletheon-extension`, and inert Executive
rollback sources consume the Corpus owner directly.

The legacy metadata-to-asset compatibility projections moved from Fabric into
Corpus because Corpus now owns the projection target. Fabric's package and asset
modules, declarations, and owner-obsolete characterization tests were deleted.
No compatibility re-export or duplicate DTO remains.

```bash
! rg -n 'fabric::types::extension_(asset|package)|super::extension_(asset|package)' crates --glob '*.rs'
! test -e crates/fabric/src/types/extension_asset.rs
! test -e crates/fabric/src/types/extension_package.rs
```

## Extension subprocess adapter retirement (2026-08-12)

- Production owner: `crates/aletheon-extension/src/subprocess.rs:1`.
- Aletheon composition now constructs the provider from that owner at
  `crates/aletheon/src/wiring/daemon/bootstrap/extensions.rs:399`.
- The retained Executive bootstrap also consumes the same owner at
  `crates/executive/src/host/daemon/bootstrap/extensions.rs:399`; it no longer
  carries a second implementation.
- `crates/executive/src/extensions` was deleted after its Rust caller count
  reached zero.
- Focused checks passed through `scripts/cargo-agent.sh` for
  `aletheon-extension --all-targets`, `executive --lib`, and
  `aletheon --all-targets`.
