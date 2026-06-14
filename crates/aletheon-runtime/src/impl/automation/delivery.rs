//! Multi-channel delivery manager for automation results.

use anyhow::{Context, Result};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use super::DeliveryTarget;

/// Manages HTTP clients keyed by destination URL origin and delivers content
/// to the appropriate channel.
pub struct DeliveryManager {
    /// Re-usable HTTP clients keyed by URL origin.
    clients: HashMap<String, reqwest::Client>,
}

impl DeliveryManager {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    /// Deliver `content` to the specified [`DeliveryTarget`].
    ///
    /// The `"[SILENT]"` marker must be stripped *before* calling this method;
    /// the caller is responsible for that check.
    pub async fn deliver(&self, target: &DeliveryTarget, content: &str) -> Result<()> {
        match target {
            DeliveryTarget::Telegram { chat_id } => {
                self.deliver_webhook(
                    &format!(
                        "https://api.telegram.org/bot{{token}}/sendMessage?chat_id={}&text={}",
                        chat_id.as_deref().unwrap_or("default"),
                        urlencoding::encode(content),
                    ),
                    content,
                )
                .await
            }
            DeliveryTarget::Discord { channel_id } => {
                info!(channel = ?channel_id, content_len = content.len(), "Discord delivery");
                // Placeholder: real implementation would POST to Discord webhook URL
                Ok(())
            }
            DeliveryTarget::Slack { channel } => {
                info!(channel = ?channel, content_len = content.len(), "Slack delivery");
                // Placeholder: real implementation would POST to Slack webhook URL
                Ok(())
            }
            DeliveryTarget::Email { address } => {
                info!(%address, content_len = content.len(), "Email delivery");
                // Placeholder: real implementation would send via SMTP
                Ok(())
            }
            DeliveryTarget::Webhook { url } => self.deliver_webhook(url, content).await,
            DeliveryTarget::Local { path } => {
                debug!(path = %path.display(), "Writing to local file");
                std::fs::write(path, content)
                    .with_context(|| format!("Failed to write to {}", path.display()))?;
                Ok(())
            }
            DeliveryTarget::Stdout => {
                println!("{}", content);
                Ok(())
            }
        }
    }

    // -- private --------------------------------------------------------------

    async fn deliver_webhook(&self, url: &str, content: &str) -> Result<()> {
        let client = reqwest::Client::new();
        let resp = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(content.to_string())
            .send()
            .await
            .context("Webhook delivery request failed")?;

        if !resp.status().is_success() {
            warn!(url, status = %resp.status(), "Webhook delivery returned non-2xx");
        } else {
            debug!(url, "Webhook delivery succeeded");
        }
        Ok(())
    }
}

impl Default for DeliveryManager {
    fn default() -> Self {
        Self::new()
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deliver_to_local_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let target = DeliveryTarget::Local {
            path: tmp.path().to_path_buf(),
        };
        let mgr = DeliveryManager::new();
        mgr.deliver(&target, "test content").await.unwrap();
        let written = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(written, "test content");
    }

    #[tokio::test]
    async fn deliver_stdout() {
        let target = DeliveryTarget::Stdout;
        let mgr = DeliveryManager::new();
        // Stdout delivery should not error.
        mgr.deliver(&target, "visible output").await.unwrap();
    }

    #[tokio::test]
    async fn deliver_empty_content() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let target = DeliveryTarget::Local {
            path: tmp.path().to_path_buf(),
        };
        let mgr = DeliveryManager::new();
        mgr.deliver(&target, "").await.unwrap();
        let written = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(written, "");
    }
}
