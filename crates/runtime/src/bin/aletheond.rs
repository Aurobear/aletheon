//! aletheond — Aletheon daemon.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "aletheond", about = "Aletheon daemon")]
struct Args {
    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Path to .env file
    #[arg(long)]
    env: Option<PathBuf>,

    /// Socket path
    #[arg(short, long, default_value = "/run/aletheond/aletheond.sock")]
    socket: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("aletheond=info")),
        )
        .init();

    let args = Args::parse();

    runtime::r#impl::daemon::run(args.config, args.env, args.socket).await
}
