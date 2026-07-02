# Rust Toolchain Compatibility Design

## Goal

Make Aletheon reproducible across Arch Linux and other development machines without tying the project to Arch's rolling Rust release.

## Compatibility Contract

- The minimum supported Rust version (MSRV) is Rust 1.85.
- Local development defaults to the pinned Rust 1.85 toolchain through `rust-toolchain.toml`.
- Arch users may continue using newer stable Rust explicitly; CI verifies that path.
- `Cargo.lock` remains committed and uses lockfile format version 4.

## Repository Changes

1. Add `rust-toolchain.toml` with Rust 1.85, `rustfmt`, and `clippy`.
2. Add `rust-version = "1.85"` to `[workspace.package]` and inherit it from every workspace package.
3. Change CI into two compatibility paths:
   - MSRV 1.85: build/check/test compatibility.
   - Current stable: check, test, clippy, rustfmt, docs, and release build.
4. Update setup documentation from the unverified `1.75.0+` claim to the explicit `1.85+` contract.

## Validation

- `cargo +1.85.0 check --workspace --all-targets`
- `cargo +1.85.0 test --workspace --all-targets`
- `cargo +stable check --workspace --all-targets`
- `cargo +stable test --workspace --all-targets`
- Confirm CI YAML distinguishes MSRV compatibility from stable quality checks.

Unix-socket tests that fail solely because a restricted execution environment forbids socket binding are tracked separately from Rust-version compatibility.

## Non-goals

- Supporting Rust versions older than 1.85.
- Pinning the project to Arch Linux's current Rust package.
- Changing dependency versions merely to silence unrelated warnings.
- Fixing Unix-socket sandbox behavior as part of this toolchain change.
