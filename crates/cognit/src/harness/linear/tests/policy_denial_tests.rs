use super::*;

#[tokio::test]
async fn authoritative_policy_denial_blocks_without_another_inference() {
    let cfg = HarnessConfig {
        max_iterations: 5,
        learning_enabled: false,
        compaction_enabled: false,
        ..HarnessConfig::default()
    };
    let mut lp = ReActLoop::new(cfg, Box::new(NoopCompressor));
    let llm = ScriptedLlm {
        calls: Mutex::new(0),
    };

    let (output, metrics) = lp
        .run(
            "delete protected file",
            &llm,
            &[],
            |_id: &str, _name: &str, _input: &serde_json::Value| async {
                (
                    "[ERROR] Policy denied: destructive managed commands are forbidden".into(),
                    true,
                )
            },
        )
        .await
        .unwrap();

    assert_eq!(metrics.stop, fabric::TurnStop::Blocked);
    assert!(!metrics.completed_normally);
    assert_eq!(metrics.tool_calls_made, 1);
    assert_eq!(metrics.tool_errors, 1);
    assert_eq!(*llm.calls.lock().unwrap(), 1);
    assert!(output.contains("Policy denied"));
}
