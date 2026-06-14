//! aletheon-cli binary — thin entry point that delegates to aletheon_body::impl::cli.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    aletheon_body::r#impl::cli::run().await
}
