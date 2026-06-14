use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(name = "agent-cli", about = "OS-Agent CLI client")]
struct Args {
    /// Socket path
    #[arg(short, long, default_value = "/tmp/agentd/agent.sock")]
    socket: PathBuf,

    /// Single message mode (non-interactive)
    #[arg(short, long)]
    message: Option<String>,

    /// Force TUI mode (default when no args)
    #[arg(long)]
    tui: bool,

    /// Force simple CLI mode (no TUI)
    #[arg(long)]
    simple: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Decide mode: --tui flag, --simple flag, or auto-detect
    let use_tui = if args.tui {
        true
    } else if args.simple || args.message.is_some() {
        false
    } else {
        // Auto-detect: use TUI if terminal supports it
        atty::is(atty::Stream::Stdout)
    };

    if use_tui {
        return aletheon_body::r#impl::ui::run(args.socket.to_str().unwrap_or("/tmp/agentd/agent.sock")).await;
    }

    // Simple CLI mode
    let mut stream = UnixStream::connect(&args.socket).await?;
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);

    if let Some(msg) = args.message {
        // Single message mode
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "chat",
            "params": { "message": msg }
        });
        let req_str = serde_json::to_string(&request)?;
        writer.write_all(req_str.as_bytes()).await?;
        writer.write_all(b"\n").await?;

        let mut response = String::new();
        reader.read_line(&mut response).await?;
        let resp: serde_json::Value = serde_json::from_str(&response)?;
        if let Some(text) = resp["result"]["response"].as_str() {
            println!("{}", text);
        } else if let Some(err) = resp["error"]["message"].as_str() {
            eprintln!("Error: {}", err);
        }
    } else {
        // Interactive CLI mode
        println!("agent-cli v0.1.0 -- Connected to agentd");
        println!("Type your message and press Enter. Type 'quit' to exit.\n");

        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let mut request_id = 0u64;

        loop {
            print!("> ");
            stdout.flush()?;

            let mut input = String::new();
            if stdin.lock().read_line(&mut input).is_err() || input.trim() == "quit" {
                break;
            }

            let trimmed = input.trim();
            if trimmed.is_empty() {
                continue;
            }

            request_id += 1;
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "chat",
                "params": { "message": trimmed }
            });

            let req_str = serde_json::to_string(&request)?;
            writer.write_all(req_str.as_bytes()).await?;
            writer.write_all(b"\n").await?;

            let mut response = String::new();
            reader.read_line(&mut response).await?;
            let resp: serde_json::Value = serde_json::from_str(&response)?;

            if let Some(text) = resp["result"]["response"].as_str() {
                println!("\n{}\n", text);
            } else if let Some(err) = resp["error"]["message"].as_str() {
                eprintln!("Error: {}\n", err);
            }
        }
    }

    Ok(())
}
