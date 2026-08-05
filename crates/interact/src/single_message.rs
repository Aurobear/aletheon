//! One-shot client adapter for the canonical typed command protocol.
//!
//! Argument parsing belongs to the top-level `aletheon` binary. This module
//! only projects an already-parsed message launch into a [`ClientIntent`] and
//! renders the authoritative daemon response.

use std::io;
use std::path::Path;

use anyhow::Result;
use fabric::protocol::client::{ClientRpcRequest, TransientApprovalDecision};
use fabric::ui_event::ClientEvent;
use fabric::Timer;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::tui::host_time::ClientTimer;
use crate::tui::response::{deduplicate_consecutive_text as deduplicate_response, format_status};

/// Default timeout for single-message mode (seconds).
const SINGLE_MESSAGE_TIMEOUT_SECS: u64 = 120;

/// Send one already-parsed prompt intent and print its terminal response.
pub(crate) async fn run(
    socket: &Path,
    message: &str,
    workspace: &fabric::WorkspacePolicy,
    requirements: Vec<fabric::TurnRequirement>,
    task_kind: Option<fabric::TaskKind>,
    session_id: Option<fabric::SessionId>,
) -> Result<()> {
    let mut stream = UnixStream::connect(socket).await?;
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);

    let request = prompt_request(message, workspace, requirements, task_kind, session_id)
        .to_json_rpc(Some(1))?;
    let req_str = serde_json::to_string(&request)?;
    writer.write_all(req_str.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let benchmark_metrics = std::env::var_os("ALETHEON_BENCHMARK_METRICS").is_some();
    let started_at = std::time::Instant::now();
    let mut tokens_in = 0u64;
    let mut tokens_out = 0u64;
    let mut cache_read_tokens = 0u64;
    let mut tool_calls = 0u64;
    let timeout_duration = std::time::Duration::from_secs(SINGLE_MESSAGE_TIMEOUT_SECS);

    let result = ClientTimer
        .timeout(timeout_duration, async {
            let mut response_buf = String::new();
            loop {
                response_buf.clear();
                match reader.read_line(&mut response_buf).await {
                    Ok(0) => {
                        eprintln!("Connection lost");
                        return Ok::<(), anyhow::Error>(());
                    }
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("Error reading response: {error}");
                        return Err(anyhow::anyhow!("Read error: {error}"));
                    }
                }

                let response: serde_json::Value = match serde_json::from_str(response_buf.trim()) {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                if response.get("method").and_then(|value| value.as_str())
                    == Some("approval_request")
                    && response.get("result").is_none()
                    && response.get("id").is_none()
                {
                    let params = &response["params"];
                    let tool = params["tool"].as_str().unwrap_or("?");
                    let action_summary = params["action_summary"].as_str().unwrap_or("");
                    let risk_level = params["risk_level"].as_str().unwrap_or("");
                    let approval_id = params["approval_id"].as_str().unwrap_or("");
                    eprintln!(
                        "\n\u{26a0}  Approval required [{risk_level}] {tool}\n   {action_summary}\n   Approve? [y]es / [a]lways / [N]o: ",
                    );
                    let mut line = String::new();
                    let decision = match io::stdin().read_line(&mut line) {
                        Ok(0) | Err(_) => TransientApprovalDecision::Deny,
                        Ok(_) => match line.trim().to_lowercase().as_str() {
                            "y" | "yes" => TransientApprovalDecision::Approve,
                            "a" | "always" => TransientApprovalDecision::ApproveForSession,
                            _ => TransientApprovalDecision::Deny,
                        },
                    };
                    let approval_response =
                        ClientRpcRequest::approval_response(approval_id, decision).to_json_rpc(None)?;
                    let response_string = serde_json::to_string(&approval_response)?;
                    writer.write_all(response_string.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    continue;
                }

                if response.get("method").and_then(|value| value.as_str()) == Some("event") {
                    if let Some(params) = response.get("params") {
                        if let Some(event) = ClientEvent::decode_if_known(params.clone()) {
                            match event {
                                ClientEvent::ToolCallStart { .. } => {}
                                ClientEvent::ToolCallComplete { tool, args, .. } => {
                                    tool_calls = tool_calls.saturating_add(1);
                                    eprintln!(
                                        "[tool] {} {}",
                                        tool,
                                        serde_json::to_string(&args).unwrap_or_default()
                                    );
                                }
                                ClientEvent::Usage { usage } => {
                                    tokens_in = tokens_in
                                        .saturating_add(usage.total_input_tokens.unwrap_or(0));
                                    tokens_out =
                                        tokens_out.saturating_add(usage.output_tokens.unwrap_or(0));
                                    cache_read_tokens = cache_read_tokens
                                        .saturating_add(usage.cache_read_tokens.unwrap_or(0));
                                }
                                ClientEvent::ToolProgress { tool, payload, .. } => {
                                    eprintln!("[tool:{tool}] {payload}");
                                }
                                _ => {}
                            }
                        }
                    }
                    continue;
                }

                if let Some(text) = response["result"]["response"].as_str() {
                    println!("{}", deduplicate_response(text));
                } else if response["result"]["queued"].as_bool() == Some(true) {
                    let prompt_id = response["result"]["prompt_id"]
                        .as_str()
                        .unwrap_or("unknown");
                    return Err(anyhow::anyhow!(
                        "request was queued as {prompt_id}; this one-shot client did not observe a terminal result"
                    ));
                } else if response["result"]["status"].is_object() {
                    println!("{}", format_status(&response["result"]["status"]));
                } else if let Some(error) = response["error"]["message"].as_str() {
                    eprintln!("Error: {error}");
                }
                if benchmark_metrics {
                    eprintln!(
                        "ALETHEON_BENCHMARK_METRICS={}",
                        serde_json::json!({
                            "total_input_tokens": tokens_in,
                            "output_tokens": tokens_out,
                            "cache_read_tokens": cache_read_tokens,
                            "latency_ms": started_at.elapsed().as_millis() as u64,
                            "tool_calls": tool_calls,
                        })
                    );
                }
                return Ok(());
            }
        })
        .await;

    match result {
        Ok(inner) => inner?,
        Err(_) => eprintln!("\n⏰ Timeout: no response after {SINGLE_MESSAGE_TIMEOUT_SECS}s"),
    }
    Ok(())
}

fn prompt_request(
    message: &str,
    workspace: &fabric::WorkspacePolicy,
    requirements: Vec<fabric::TurnRequirement>,
    task_kind: Option<fabric::TaskKind>,
    session_id: Option<fabric::SessionId>,
) -> ClientRpcRequest {
    let permission_mode = crate::host::permission_mode_from_environment();
    let explicit_session = session_id.map(|session_id| session_id.0).or_else(|| {
        std::env::var("ALETHEON_BENCHMARK_SESSION_ID")
            .ok()
            .filter(|session_id| !session_id.trim().is_empty())
    });
    crate::intent::rpc(prompt_intent(
        message,
        workspace,
        requirements,
        task_kind,
        permission_mode,
        explicit_session,
    ))
}

fn prompt_intent(
    message: &str,
    workspace: &fabric::WorkspacePolicy,
    requirements: Vec<fabric::TurnRequirement>,
    task_kind: Option<fabric::TaskKind>,
    permission_mode: fabric::permission::HostPermissionMode,
    explicit_session: Option<String>,
) -> fabric::contract::command::ClientIntent {
    let session_id = explicit_session.map_or_else(
        || fabric::SessionId(format!("message-{}", uuid::Uuid::new_v4())),
        fabric::SessionId,
    );
    crate::intent::submit_prompt(crate::intent::PromptIntent {
        surface: fabric::contract::command::ClientSurface::Cli,
        correlation_id: format!("cli-message:{}", uuid::Uuid::new_v4()),
        content: message,
        session_id: Some(session_id),
        workspace,
        requirements,
        task_kind,
        permission_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_messages_use_fresh_sessions_unless_explicitly_overridden() {
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), Vec::new()).unwrap();
        let fresh = prompt_intent(
            "hello",
            &workspace,
            Vec::new(),
            None,
            fabric::permission::HostPermissionMode::Safe,
            None,
        );
        let explicit = prompt_intent(
            "hello",
            &workspace,
            Vec::new(),
            None,
            fabric::permission::HostPermissionMode::Safe,
            Some("shared-session".into()),
        );

        assert!(matches!(
            fresh,
            fabric::contract::command::ClientIntent {
                command: fabric::contract::command::ClientCommand::SubmitPrompt(
                    fabric::contract::command::SubmitPromptIntent {
                        session_id: Some(fabric::SessionId(id)),
                        ..
                    }
                ),
                ..
            } if id.starts_with("message-")
        ));
        assert!(matches!(
            explicit,
            fabric::contract::command::ClientIntent {
                command: fabric::contract::command::ClientCommand::SubmitPrompt(
                    fabric::contract::command::SubmitPromptIntent {
                        session_id: Some(fabric::SessionId(id)),
                        ..
                    }
                ),
                ..
            } if id == "shared-session"
        ));
    }

    #[test]
    fn one_shot_adapter_does_not_reparse_slash_syntax() {
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), Vec::new()).unwrap();
        let intent = prompt_intent(
            "/status",
            &workspace,
            Vec::new(),
            None,
            fabric::permission::HostPermissionMode::Safe,
            None,
        );

        assert!(matches!(
            intent.command,
            fabric::contract::command::ClientCommand::SubmitPrompt(
                fabric::contract::command::SubmitPromptIntent { content, .. }
            ) if content == "/status"
        ));
    }
}
