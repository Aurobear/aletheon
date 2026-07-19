#![allow(deprecated)]
//! Basic Agent Example
//!
//! Minimal example demonstrating that the Aletheon MacroKernel initializes
//! correctly. Production turn execution uses the daemon JSON-RPC interface
//! or the `aletheon exec` CLI path.
//!
//! Run with:  cargo run -p basic_agent

use anyhow::Result;
use executive::{AletheonExecutive, ExecutiveConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let config = ExecutiveConfig::default();
    let _runtime = AletheonExecutive::new(config);

    println!("Aletheon MacroKernel initialized. Daemon mode is the primary interface.");
    println!("Run `aletheon daemon` to start the JSON-RPC server.");
    println!("For direct turn execution, use `aletheon exec <prompt>`.");
    Ok(())
}
