# AK2-25 contracts package rename evidence

Date: 2026-08-12

The former `fabric` package and `crates/fabric` path were renamed mechanically to the
`contracts` package at `crates/contracts`. All workspace Cargo dependencies and Rust paths now use
`contracts`; no `fabric` package, `fabric::` Rust path, `crates/fabric` path, or Cargo dependency
alias remains. The distinct `fabric-client` package is not a compatibility alias for the contracts
crate and remains unchanged.

The temporary D1 `contracts` module was deleted during the package rename because keeping
`contracts::contracts` would create an ambiguous compatibility namespace after the crate itself
became `contracts`.

Validation evidence:

- `bash scripts/cargo-agent.sh check --workspace --all-targets` passed.
- `ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture` passed with the four
  pre-existing reviewed host-effect findings and no additions.
- `bash scripts/cargo-agent.sh fmt --all -- --check` and `git diff --check` passed.
- A repository scan found no old package/path/Rust identifier; only the independently named
  `fabric-client` package remains.
