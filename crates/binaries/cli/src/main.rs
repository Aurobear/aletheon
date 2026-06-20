//! aletheon-cli — thin binary that delegates to interact::cli.
//!
//! The CLI logic lives in `aletheon-body/src/impl/cli/`. This crate exists
//! for backward compatibility so that `cargo install aletheon-cli` still works.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    interact::cli::run().await
}
