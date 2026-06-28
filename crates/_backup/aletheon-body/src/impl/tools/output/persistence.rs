use std::path::PathBuf;
use anyhow::Result;
use tracing::debug;

use super::config::OutputConfig;
use super::truncation::truncate_head_tail;

#[derive(Debug, Clone)]
pub enum ProcessedOutput {
    Inline {
        content: String,
        original_bytes: usize,
    },
    Overflow {
        summary: String,
        overflow_path: PathBuf,
        original_bytes: usize,
        total_chars: usize,
    },
}

impl ProcessedOutput {
    pub fn to_context_content(&self) -> &str {
        match self {
            ProcessedOutput::Inline { content, .. } => content,
            ProcessedOutput::Overflow { summary, .. } => summary,
        }
    }

    pub fn was_truncated(&self) -> bool {
        matches!(self, ProcessedOutput::Overflow { .. })
    }
}

pub async fn process_result(
    tool_name: &str,
    content: &str,
    config: &OutputConfig,
) -> Result<ProcessedOutput> {
    // Check pinned threshold — never persist for these tools
    if let Some(&pinned) = config.pinned_thresholds.get(tool_name) {
        if content.len() <= pinned {
            return Ok(ProcessedOutput::Inline {
                content: content.to_string(),
                original_bytes: content.len(),
            });
        }
    }

    let threshold = config
        .tool_overrides
        .get(tool_name)
        .copied()
        .unwrap_or(config.max_output_chars);

    if content.len() <= threshold {
        return Ok(ProcessedOutput::Inline {
            content: content.to_string(),
            original_bytes: content.len(),
        });
    }

    // Overflow to file
    tokio::fs::create_dir_all(&config.overflow_dir).await?;
    let filename = format!(
        "tool_output_{}_{}.txt",
        tool_name,
        chrono::Utc::now().timestamp_millis()
    );
    let path = config.overflow_dir.join(&filename);
    tokio::fs::write(&path, content).await?;

    debug!(
        tool = tool_name,
        bytes = content.len(),
        path = %path.display(),
        "Tool output overflowed to file"
    );

    let truncated = truncate_head_tail(content, &config.truncation);
    let summary = format!(
        "{}\n[Full output: {} chars, saved to {}]",
        truncated.content,
        content.len(),
        path.display()
    );

    Ok(ProcessedOutput::Overflow {
        summary,
        overflow_path: path,
        original_bytes: content.len(),
        total_chars: content.chars().count(),
    })
}

pub async fn cleanup_overflow_dir(config: &OutputConfig) -> Result<usize> {
    let mut removed = 0;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(config.retention_days as i64);

    let mut entries = tokio::fs::read_dir(&config.overflow_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if let Ok(metadata) = entry.metadata().await {
            if let Ok(modified) = metadata.modified() {
                let modified_dt: chrono::DateTime<chrono::Utc> = modified.into();
                if modified_dt < cutoff {
                    tokio::fs::remove_file(entry.path()).await?;
                    removed += 1;
                }
            }
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_inline_when_under_threshold() {
        let tmp = TempDir::new().unwrap();
        let config = OutputConfig {
            max_output_chars: 1000,
            overflow_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let result = process_result("bash_exec", "short output", &config)
            .await
            .unwrap();
        assert!(matches!(result, ProcessedOutput::Inline { .. }));
    }

    #[tokio::test]
    async fn test_overflow_when_over_threshold() {
        let tmp = TempDir::new().unwrap();
        let config = OutputConfig {
            max_output_chars: 100,
            overflow_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let long_output = "x".repeat(200);
        let result = process_result("bash_exec", &long_output, &config)
            .await
            .unwrap();
        assert!(matches!(result, ProcessedOutput::Overflow { .. }));
        let entries: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn test_pinned_threshold_never_persists() {
        let tmp = TempDir::new().unwrap();
        let config = OutputConfig {
            max_output_chars: 100,
            overflow_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let long_output = "x".repeat(200);
        let result = process_result("file_read", &long_output, &config)
            .await
            .unwrap();
        assert!(matches!(result, ProcessedOutput::Inline { .. }));
    }
}
