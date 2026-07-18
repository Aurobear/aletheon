//! Web fetch tool — HTTP requests with response capping.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024; // 1 MB

pub struct WebFetchTool {
    network_policy: Option<Arc<fabric::network_policy::NetworkPolicy>>,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            network_policy: None,
        }
    }

    pub fn with_network_policy(mut self, policy: fabric::network_policy::NetworkPolicy) -> Self {
        self.network_policy = Some(Arc::new(policy));
        self
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL and return the response body. Supports GET and POST methods. Response is capped at 1 MB."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST"],
                    "description": "HTTP method (default: GET)"
                },
                "body": {
                    "type": "string",
                    "description": "Request body for POST requests"
                }
            },
            "required": ["url"]
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

        let url = match input.get("url").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => {
                return ToolResult {
                    content: "Error: 'url' parameter is required".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        // Validate against network policy before making the request.
        if let Some(ref policy) = self.network_policy {
            if let Err(reason) = policy.allows_url(&url) {
                return ToolResult {
                    content: format!("Error: Network policy blocked URL: {}", reason),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        }

        let method = input
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_uppercase();

        let body = input
            .get("body")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let client = reqwest::Client::new();

        let request = match method.as_str() {
            "GET" => client.get(&url),
            "POST" => {
                let mut req = client.post(&url);
                if let Some(ref b) = body {
                    req = req
                        .header("content-type", "application/json")
                        .body(b.clone());
                }
                req
            }
            _ => {
                return ToolResult {
                    content: format!("Error: unsupported method '{}'. Use GET or POST.", method),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        let elapsed = ctx.clock.mono_now().0.saturating_sub(start.0);

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                let is_error = !status.is_success();

                match response.bytes().await {
                    Ok(bytes) => {
                        let truncated = bytes.len() > MAX_RESPONSE_BYTES;
                        let body_bytes = if truncated {
                            &bytes[..MAX_RESPONSE_BYTES]
                        } else {
                            &bytes
                        };

                        let content = match std::str::from_utf8(body_bytes) {
                            Ok(s) => {
                                if truncated {
                                    format!(
                                        "[Status: {}] (response truncated to {} bytes)\n{}",
                                        status.as_u16(),
                                        MAX_RESPONSE_BYTES,
                                        s
                                    )
                                } else {
                                    format!("[Status: {}]\n{}", status.as_u16(), s)
                                }
                            }
                            Err(_) => format!(
                                "[Status: {}] (binary response, {} bytes, not shown)",
                                status.as_u16(),
                                bytes.len()
                            ),
                        };

                        ToolResult {
                            content,
                            is_error,
                            metadata: ToolResultMeta {
                                execution_time_ms: elapsed,
                                truncated,
                            },
                        }
                    }
                    Err(e) => ToolResult {
                        content: format!("Error reading response body: {}", e),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: elapsed,
                            truncated: false,
                        },
                    },
                }
            }
            Err(e) => ToolResult {
                content: format!("Error fetching URL '{}': {}", url, e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
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
        let tool = WebFetchTool::new();
        assert_eq!(tool.name(), "web_fetch");
        assert_eq!(tool.permission_level(), PermissionLevel::L1);
    }

    #[test]
    fn test_input_schema() {
        let tool = WebFetchTool::new();
        let schema = tool.input_schema();

        // Verify required fields
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.contains(&json!("url")));

        // Verify properties exist
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("url"));
        assert!(props.contains_key("method"));
        assert!(props.contains_key("body"));
    }

    #[tokio::test]
    async fn test_missing_url() {
        let tool = WebFetchTool::new();
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
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("required"));
    }

    #[tokio::test]
    async fn test_unsupported_method() {
        let tool = WebFetchTool::new();
        let input = json!({
            "url": "http://example.com",
            "method": "DELETE"
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
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("unsupported method"));
    }

    #[tokio::test]
    async fn test_network_policy_blocks_denied_host() {
        let policy = NetworkPolicy {
            deny_hosts: vec!["evil.com".into()],
            allow_dns: true,
            ..Default::default()
        };
        let tool = WebFetchTool::new().with_network_policy(policy);
        let input = json!({
            "url": "https://evil.com/path",
            "method": "GET"
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
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("Network policy blocked URL"));
    }

    #[tokio::test]
    async fn test_network_policy_allows_clearn_url() {
        let policy = NetworkPolicy {
            allow_hosts: vec!["httpbin.org".into()],
            allow_dns: true,
            ..Default::default()
        };
        let tool = WebFetchTool::new().with_network_policy(policy);
        let input = json!({
            "url": "http://httpbin.org/get",
            "method": "GET"
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
                },
            )
            .await;

        // Should not be a network policy error — it should either make the request
        // (possibly failing due to DNS/network) or succeed. The key is it's not blocked.
        assert!(
            !result.content.contains("Network policy blocked URL"),
            "allowed URL was blocked by policy: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_network_policy_none_allows_all() {
        let tool = WebFetchTool::new(); // no policy
        let input = json!({
            "url": "https://some-random-site-12345.example",
            "method": "GET"
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
                },
            )
            .await;

        // When no policy is set, the request should not be blocked by policy.
        // It may fail due to DNS/network, but the error message should not mention policy.
        assert!(
            !result.content.contains("Network policy blocked"),
            "no-policy should not block: {}",
            result.content
        );
    }
}
