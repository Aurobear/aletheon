//! aletheon-cli — thin binary that delegates to aletheon_body::impl::cli.
//!
//! The CLI logic lives in `aletheon-body/src/impl/cli/`. This crate exists
//! for backward compatibility so that `cargo install aletheon-cli` still works.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    aletheon_body::r#impl::cli::run().await
}
