//! aletheon-cli binary — thin entry point that delegates to corpus::impl::cli.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    corpus::r#impl::cli::run().await
}
