use super::config::{OutputConfig, TurnBudgetConfig};
use super::persistence::{process_result, ProcessedOutput};

pub async fn enforce_turn_budget(
    results: &mut Vec<(String, ProcessedOutput)>,
    output_config: &OutputConfig,
    budget_config: &TurnBudgetConfig,
) -> anyhow::Result<()> {
    let total_inline: usize = results
        .iter()
        .map(|(_, r)| match r {
            ProcessedOutput::Inline { content, .. } => content.len(),
            ProcessedOutput::Overflow { summary, .. } => summary.len(),
        })
        .sum();

    if total_inline <= budget_config.turn_budget_chars {
        return Ok(());
    }

    let mut inline_candidates: Vec<(usize, usize)> = results
        .iter()
        .enumerate()
        .filter_map(|(i, (_, r))| match r {
            ProcessedOutput::Inline { original_bytes, .. } => Some((i, *original_bytes)),
            _ => None,
        })
        .collect();
    inline_candidates.sort_by(|a, b| b.1.cmp(&a.1));

    let mut current_total = total_inline;
    for (idx, _) in inline_candidates {
        if current_total <= budget_config.turn_budget_chars {
            break;
        }

        let (tool_name, output) = &results[idx];
        if let ProcessedOutput::Inline {
            content,
            original_bytes,
        } = output
        {
            let tool_name = tool_name.clone();
            let content = content.clone();
            let original_bytes = *original_bytes;

            let new_result = process_result(&tool_name, &content, output_config).await?;
            let saved = match &new_result {
                ProcessedOutput::Inline { content, .. } => content.len(),
                ProcessedOutput::Overflow { summary, .. } => summary.len(),
            };

            current_total = current_total - original_bytes + saved;
            results[idx].1 = new_result;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_no_enforcement_when_under_budget() {
        let tmp = TempDir::new().unwrap();
        let output_config = OutputConfig {
            overflow_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let budget_config = TurnBudgetConfig {
            turn_budget_chars: 1000,
            preview_chars: 100,
        };

        let mut results = vec![(
            "bash_exec".to_string(),
            ProcessedOutput::Inline {
                content: "short".to_string(),
                original_bytes: 5,
            },
        )];

        enforce_turn_budget(&mut results, &output_config, &budget_config)
            .await
            .unwrap();
        assert!(matches!(&results[0].1, ProcessedOutput::Inline { .. }));
    }

    #[tokio::test]
    async fn test_persists_largest_result_first() {
        let tmp = TempDir::new().unwrap();
        let output_config = OutputConfig {
            max_output_chars: 50,
            overflow_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let budget_config = TurnBudgetConfig {
            turn_budget_chars: 100,
            preview_chars: 50,
        };

        let mut results = vec![
            (
                "tool_a".to_string(),
                ProcessedOutput::Inline {
                    content: "x".repeat(80),
                    original_bytes: 80,
                },
            ),
            (
                "tool_b".to_string(),
                ProcessedOutput::Inline {
                    content: "y".repeat(60),
                    original_bytes: 60,
                },
            ),
        ];

        enforce_turn_budget(&mut results, &output_config, &budget_config)
            .await
            .unwrap();
        assert!(results[0].1.was_truncated());
    }
}
