//! aletheon — interactive TUI and CLI for Aletheon agent.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    interact::cli::run().await
}
