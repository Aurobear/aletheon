use super::*;

struct ProjectionLlm {
    calls: Mutex<usize>,
    visible_tools: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl LlmProvider for ProjectionLlm {
    async fn complete(
        &self,
        _messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.visible_tools
            .lock()
            .unwrap()
            .push(tools.iter().map(|tool| tool.name.clone()).collect());
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            Ok(LlmResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "discover-1".into(),
                    name: "tool_search".into(),
                    input: serde_json::json!({"query": "repo inspection"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: InferenceUsage::default(),
            })
        } else {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "The newly activated repository tool is now model-visible.".into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: InferenceUsage::default(),
            })
        }
    }

    async fn complete_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unimplemented!("collecting adapter uses complete")
    }

    fn name(&self) -> &str {
        "projection"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

#[tokio::test]
async fn discovered_tool_schema_is_visible_on_the_next_inference_round() {
    let mut loop_state = ReActLoop::new(
        HarnessConfig {
            max_iterations: 3,
            learning_enabled: false,
            compaction_enabled: false,
            ..HarnessConfig::default()
        },
        Box::new(NoopCompressor),
    );
    let llm = ProjectionLlm {
        calls: Mutex::new(0),
        visible_tools: Mutex::new(Vec::new()),
    };
    let initial = ToolDefinition {
        name: "tool_search".into(),
        description: "discover authorized tools".into(),
        input_schema: serde_json::json!({"type": "object"}),
    };
    let activated = ToolDefinition {
        name: "repo_inspect".into(),
        description: "inspect a repository".into(),
        input_schema: serde_json::json!({"type": "object"}),
    };

    let (_output, metrics) = loop_state
        .run(
            "inspect the repository",
            &llm,
            &[initial],
            move |_id: &str, _name: &str, _input: &serde_json::Value| {
                let activated = activated.clone();
                async move {
                    crate::harness::event_sink::ToolResultEvent {
                        content: r#"{"ok":true,"matches":[{"name":"repo_inspect"}]}"#.into(),
                        is_error: false,
                        execution_time_ms: 1,
                        patch_delta: None,
                        activated_tool_definitions: vec![activated],
                    }
                }
            },
        )
        .await
        .unwrap();

    let visible_tools = llm.visible_tools.lock().unwrap();
    assert_eq!(visible_tools[0], vec!["tool_search"]);
    assert_eq!(visible_tools[1], vec!["tool_search", "repo_inspect"]);
    assert_eq!(metrics.iterations, 2);
    assert_eq!(metrics.tool_calls_made, 1);
}
