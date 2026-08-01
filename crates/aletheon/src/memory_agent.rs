//! Independent Memory Agent scheduler. All authority remains behind the daemon
//! protocol; this process never opens Aletheon or supplemental persistence.

use std::time::Duration;

const ONESHOT_ENV: &str = "ALETHEON_MEMORY_AGENT_ONESHOT";
const MAX_ITEMS_ENV: &str = "ALETHEON_MEMORY_AGENT_MAX_ITEMS";
const DRY_RUN_ENV: &str = "ALETHEON_MEMORY_AGENT_DRY_RUN";

pub async fn serve_official_user_socket() -> anyhow::Result<()> {
    let one_shot = std::env::var_os(ONESHOT_ENV).is_some();
    let max_items = std::env::var(MAX_ITEMS_ENV)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(20)
        .clamp(1, 64);
    let dry_run = std::env::var_os(DRY_RUN_ENV).is_some();
    let mut backoff = Duration::from_secs(1);
    loop {
        match interact::memory_client::MemoryAgentClient::connect_official(None).await {
            Ok(mut client) => {
                backoff = Duration::from_secs(1);
                loop {
                    let request_id = format!("memory-agent:{}", uuid::Uuid::new_v4());
                    match client.run(request_id, max_items, dry_run).await {
                        Ok(receipt) => {
                            if one_shot {
                                println!("{}", serde_json::to_string_pretty(&receipt)?);
                                return Ok(());
                            }
                            let delay = if receipt.claimed == 0 {
                                Duration::from_secs(30)
                            } else {
                                Duration::from_secs(1)
                            };
                            tokio::select! {
                                _ = tokio::signal::ctrl_c() => return Ok(()),
                                _ = tokio::time::sleep(delay) => {}
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Memory Agent request failed; reconnecting");
                            if one_shot {
                                return Err(error);
                            }
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, delay_ms = backoff.as_millis(), "Memory Agent daemon connection unavailable");
                if one_shot {
                    return Err(error);
                }
            }
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = backoff.saturating_mul(2).min(Duration::from_secs(60));
    }
}

pub async fn run_once(max_items: u16, dry_run: bool) -> anyhow::Result<()> {
    anyhow::ensure!((1..=64).contains(&max_items), "max-items must be 1..=64");
    let executable = std::env::current_exe()?;
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("memory-agent")
        .arg("serve")
        .arg("--official-user-socket")
        .env(ONESHOT_ENV, "1")
        .env(MAX_ITEMS_ENV, max_items.to_string());
    if dry_run {
        command.env(DRY_RUN_ENV, "1");
    }
    let status = command.status().await?;
    anyhow::ensure!(status.success(), "one-shot Memory Agent failed: {status}");
    Ok(())
}
