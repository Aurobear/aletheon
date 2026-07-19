//! Web search tool — search via an external API configured through environment variables.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct WebSearchTool {
    network_policy: Arc<fabric::network_policy::NetworkPolicy>,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            network_policy: Arc::new(fabric::network_policy::NetworkPolicy::default()),
        }
    }

    pub fn with_network_policy(mut self, policy: fabric::network_policy::NetworkPolicy) -> Self {
        self.network_policy = Arc::new(policy);
        self
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web using an external search API. Requires SEARCH_API_URL and SEARCH_API_KEY environment variables to be configured."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 10)"
                }
            },
            "required": ["query"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(Self {
            network_policy: self.network_policy.clone(),
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.to_string(),
            None => {
                return ToolResult {
                    content: "Error: 'query' parameter is required".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        };

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        // Read configuration from environment
        let api_url = match std::env::var("SEARCH_API_URL") {
            Ok(u) => u,
            Err(_) => {
                return ToolResult {
                    content: "Error: SEARCH_API_URL environment variable is not set. Configure it to enable web search.".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        };

        let api_key = match std::env::var("SEARCH_API_KEY") {
            Ok(k) => k,
            Err(_) => {
                return ToolResult {
                    content: "Error: SEARCH_API_KEY environment variable is not set. Configure it to enable web search.".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        };

        // Validate the API URL against network policy before making the request.
        {
            if let Err(reason) = self.network_policy.allows_url(&api_url) {
                return ToolResult {
                    content: format!("Error: Network policy blocked URL: {}", reason),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        }

        let client = reqwest::Client::new();
        let body = json!({
            "query": query,
            "max_results": max_results
        });

        let elapsed = ctx.clock.mono_now().0.saturating_sub(start.0);

        match client
            .post(&api_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                let is_error = !status.is_success();

                match response.text().await {
                    Ok(text) => ToolResult {
                        content: format!("[Status: {}]\n{}", status.as_u16(), text),
                        is_error,
                        metadata: ToolResultMeta {
                            execution_time_ms: elapsed,
                            truncated: false,
                            patch_delta: None,
                        },
                    },
                    Err(e) => ToolResult {
                        content: format!("Error reading search response: {}", e),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: elapsed,
                            truncated: false,
                            patch_delta: None,
                        },
                    },
                }
            }
            Err(e) => ToolResult {
                content: format!("Error executing search request: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                    patch_delta: None,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::network_policy::NetworkPolicy;

    #[test]
    fn test_tool_metadata() {
        let tool = WebSearchTool::new();
        assert_eq!(tool.name(), "web_search");
        assert_eq!(tool.permission_level(), PermissionLevel::L1);
    }

    #[test]
    fn test_input_schema() {
        let tool = WebSearchTool::new();
        let schema = tool.input_schema();

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.contains(&json!("query")));

        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("max_results"));
    }

    #[tokio::test]
    async fn test_missing_query() {
        let tool = WebSearchTool::new();
        let input = json!({});
        let tmp = tempfile::tempdir().unwrap();

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("required"));
    }

    #[tokio::test]
    async fn test_missing_env_vars() {
        // Ensure env vars are not set for this test
        std::env::remove_var("SEARCH_API_URL");
        std::env::remove_var("SEARCH_API_KEY");

        let tool = WebSearchTool::new().with_network_policy(NetworkPolicy {
            default_action: fabric::network_policy::NetworkDefaultAction::Allow,
            allow_dns: true,
            ..Default::default()
        });
        let input = json!({
            "query": "test query"
        });
        let tmp = tempfile::tempdir().unwrap();

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(result.is_error);
        assert!(
            result.content.contains("SEARCH_API_URL") || result.content.contains("SEARCH_API_KEY"),
            "Expected env var error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_network_policy_blocks_denied_host() {
        let policy = NetworkPolicy {
            deny_hosts: vec!["evil-search.com".into()],
            allow_dns: true,
            ..Default::default()
        };
        let tool = WebSearchTool::new().with_network_policy(policy);
        let input = json!({
            "query": "test"
        });
        let tmp = tempfile::tempdir().unwrap();

        // Set env var to a denied host
        std::env::set_var("SEARCH_API_URL", "https://evil-search.com/api/search");
        std::env::set_var("SEARCH_API_KEY", "dummy-key");

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("Network policy blocked URL"));

        // Clean up env vars
        std::env::remove_var("SEARCH_API_URL");
        std::env::remove_var("SEARCH_API_KEY");
    }

    #[tokio::test]
    async fn test_default_network_policy_denies_all() {
        let tool = WebSearchTool::new();
        let input = json!({
            "query": "test query"
        });
        let tmp = tempfile::tempdir().unwrap();

        // Set a valid-looking URL; the default policy must reject it before I/O.
        std::env::set_var(
            "SEARCH_API_URL",
            "https://some-random-search-api-12345.example/search",
        );
        std::env::set_var("SEARCH_API_KEY", "dummy-key");

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(
            result.content.contains("Network policy blocked"),
            "default policy should block: {}",
            result.content
        );

        // Clean up env vars
        std::env::remove_var("SEARCH_API_URL");
        std::env::remove_var("SEARCH_API_KEY");
    }
}
