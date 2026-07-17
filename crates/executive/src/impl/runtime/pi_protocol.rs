//! Strict adapters for Pi's LF-delimited JSON and RPC protocols.

use anyhow::{bail, Context, Result};
use fabric::{AttemptEvidence, AttemptUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_TOOL_EVENTS: usize = 128;
const MAX_TOOL_EVIDENCE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedPiOutput {
    pub session_id: String,
    pub final_text: String,
    pub usage: AttemptUsage,
    pub evidence: Vec<AttemptEvidence>,
}

/// Parse one complete Pi `--mode json` stream.
///
/// The parser deliberately rejects unknown event types: new privileged Pi
/// protocol surfaces must be reviewed before Aletheon accepts them.
pub fn parse_job_jsonl(input: &str, expected_version: u32) -> Result<ParsedPiOutput> {
    let mut records = input.split('\n');
    let header_line = records.next().context("Pi JSON stream is empty")?;
    if header_line.is_empty() {
        bail!("Pi JSON stream does not start with a session header");
    }
    let header: Value = serde_json::from_str(header_line).context("invalid Pi session header")?;
    if string_field(&header, "type")? != "session" {
        bail!("Pi JSON stream does not start with a session header");
    }
    let version = header
        .get("version")
        .and_then(Value::as_u64)
        .context("Pi session header lacks numeric version")?;
    if version != u64::from(expected_version) {
        bail!("unsupported Pi JSON protocol version {version}; expected {expected_version}");
    }
    let session_id = string_field(&header, "id")?.to_owned();
    if session_id.is_empty() {
        bail!("Pi session header has an empty id");
    }

    let mut started = false;
    let mut ended = false;
    let mut final_text = None;
    let mut usage = AttemptUsage::default();
    let mut message_usage_seen = false;
    let mut evidence = Vec::new();

    for (offset, raw) in records.enumerate() {
        if raw.is_empty() && offset == input.split('\n').count().saturating_sub(2) {
            continue;
        }
        if raw.is_empty() {
            bail!("blank record inside Pi JSON stream");
        }
        if ended {
            bail!("Pi JSON stream contains records after agent_end");
        }
        let event: Value = serde_json::from_str(raw)
            .with_context(|| format!("invalid Pi JSON event at record {}", offset + 2))?;
        match string_field(&event, "type")? {
            "agent_start" => {
                if started {
                    bail!("Pi JSON stream contains duplicate agent_start");
                }
                started = true;
            }
            "agent_end" => {
                if !started {
                    bail!("Pi agent_end precedes agent_start");
                }
                final_text = final_text.or_else(|| last_assistant_text(event.get("messages")));
                ended = true;
            }
            "message_end" => {
                if let Some(message) = event.get("message") {
                    if message.get("role").and_then(Value::as_str) == Some("assistant") {
                        if let Some(text) = message_text(message) {
                            final_text = Some(text);
                        }
                        if message.get("usage").is_some() {
                            accumulate_usage(message.get("usage"), &mut usage);
                            message_usage_seen = true;
                        }
                    }
                }
            }
            "turn_end" => {
                if let Some(message) = event.get("message") {
                    // Pi repeats the terminal assistant message in `turn_end`.
                    // Treat its usage as a fallback rather than double-counting it.
                    if !message_usage_seen {
                        accumulate_usage(message.get("usage"), &mut usage);
                    }
                }
            }
            "tool_execution_end" => {
                if evidence.len() >= MAX_TOOL_EVENTS {
                    bail!("Pi JSON stream exceeds the tool evidence limit");
                }
                let tool_name = string_field(&event, "toolName")?;
                let tool_call_id = string_field(&event, "toolCallId")?;
                let is_error = event
                    .get("isError")
                    .and_then(Value::as_bool)
                    .context("Pi tool_execution_end lacks isError")?;
                evidence.push(
                    AttemptEvidence {
                        kind: "pi_tool_execution".into(),
                        summary: format!(
                            "Pi tool {tool_name} ({tool_call_id}) {}",
                            if is_error { "failed" } else { "completed" }
                        ),
                        content: serde_json::to_string(
                            event.get("result").unwrap_or(&Value::Null),
                        )?,
                    }
                    .bounded_for_persistence(MAX_TOOL_EVIDENCE_BYTES),
                );
            }
            "turn_start"
            | "message_start"
            | "message_update"
            | "tool_execution_start"
            | "tool_execution_update"
            | "queue_update"
            | "compaction_start"
            | "compaction_end"
            | "auto_retry_start"
            | "auto_retry_end" => {}
            event_type => bail!("unsupported Pi JSON event type: {event_type}"),
        }
    }

    if !started || !ended {
        bail!("truncated Pi JSON stream: missing agent lifecycle terminator");
    }
    let final_text = final_text
        .filter(|text| !text.trim().is_empty())
        .context("Pi JSON stream has no terminal assistant text")?;
    Ok(ParsedPiOutput {
        session_id,
        final_text,
        usage,
        evidence,
    })
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("Pi JSON record lacks string field {field}"))
}

fn last_assistant_text(messages: Option<&Value>) -> Option<String> {
    messages?.as_array()?.iter().rev().find_map(|message| {
        (message.get("role").and_then(Value::as_str) == Some("assistant"))
            .then(|| message_text(message))
            .flatten()
    })
}

fn message_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }
    let text = content
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<String>();
    (!text.is_empty()).then_some(text)
}

fn accumulate_usage(value: Option<&Value>, usage: &mut AttemptUsage) {
    let Some(value) = value else { return };
    usage.input_tokens = usage.input_tokens.saturating_add(number_field(
        value,
        &["inputTokens", "input_tokens", "input"],
    ));
    usage.output_tokens = usage.output_tokens.saturating_add(number_field(
        value,
        &["outputTokens", "output_tokens", "output"],
    ));
    let cost = value
        .get("cost")
        .and_then(|cost| cost.get("total").or_else(|| cost.get("totalCost")))
        .or_else(|| value.get("costUsd"))
        .and_then(Value::as_f64);
    if let Some(cost) = cost {
        usage.cost_usd = Some(usage.cost_usd.unwrap_or_default() + cost);
    }
}

fn number_field(value: &Value, names: &[&str]) -> u64 {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_u64))
        .unwrap_or_default()
}

/// Commands supported by the reviewed Pi RPC boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiRpcCommand {
    Prompt { id: String, message: String },
    Steer { id: String, message: String },
    FollowUp { id: String, message: String },
    Abort { id: String },
    GetState { id: String },
}

impl PiRpcCommand {
    pub fn id(&self) -> &str {
        match self {
            Self::Prompt { id, .. }
            | Self::Steer { id, .. }
            | Self::FollowUp { id, .. }
            | Self::Abort { id }
            | Self::GetState { id } => id,
        }
    }

    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::Abort { .. } => "abort",
            Self::GetState { .. } => "get_state",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PiRpcRecord {
    Response {
        id: String,
        command: String,
        success: bool,
        data: Option<Value>,
        error: Option<String>,
    },
    Event(Value),
}

/// Parse exactly one LF-terminated Pi RPC record.
///
/// Unknown event types, CRLF, multiple records and privileged extension UI
/// requests are rejected so protocol drift cannot silently gain authority.
pub fn parse_rpc_record(record: &[u8]) -> Result<PiRpcRecord> {
    if !record.ends_with(b"\n") || record[..record.len().saturating_sub(1)].contains(&b'\n') {
        bail!("Pi RPC record is not exactly one LF-framed JSON object");
    }
    let payload = &record[..record.len() - 1];
    if payload.contains(&b'\r') {
        bail!("Pi RPC record uses unsupported CR framing");
    }
    let value: Value = serde_json::from_slice(payload).context("invalid Pi RPC JSON record")?;
    let record_type = string_field(&value, "type")?;
    if record_type == "response" {
        return Ok(PiRpcRecord::Response {
            id: string_field(&value, "id")?.to_owned(),
            command: string_field(&value, "command")?.to_owned(),
            success: value
                .get("success")
                .and_then(Value::as_bool)
                .context("Pi RPC response lacks boolean success")?,
            data: value.get("data").cloned(),
            error: value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
    match record_type {
        "agent_start"
        | "agent_end"
        | "agent_settled"
        | "turn_start"
        | "turn_end"
        | "message_start"
        | "message_update"
        | "message_end"
        | "tool_execution_start"
        | "tool_execution_update"
        | "tool_execution_end"
        | "queue_update"
        | "compaction_start"
        | "compaction_end"
        | "auto_retry_start"
        | "auto_retry_end" => Ok(PiRpcRecord::Event(value)),
        other => bail!("unsupported Pi RPC event type: {other}"),
    }
}

pub fn validate_rpc_response(record: PiRpcRecord, command: &PiRpcCommand) -> Result<Option<Value>> {
    let PiRpcRecord::Response {
        id,
        command: actual_command,
        success,
        data,
        error,
    } = record
    else {
        bail!("expected Pi RPC command response");
    };
    if id != command.id() || actual_command != command.command_name() {
        bail!(
            "Pi RPC response correlation drift: expected {}/{}, got {}/{}",
            command.id(),
            command.command_name(),
            id,
            actual_command
        );
    }
    if !success {
        bail!(
            "Pi RPC command {} was rejected: {}",
            command.command_name(),
            error.unwrap_or_else(|| "unspecified error".into())
        );
    }
    Ok(data)
}

impl PiRpcCommand {
    pub fn to_jsonl(&self) -> Result<String> {
        let encoded = serde_json::to_string(self)?;
        if encoded.contains(['\n', '\r']) {
            bail!("Pi RPC command violated LF-only framing");
        }
        Ok(format!("{encoded}\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(events: &[&str]) -> String {
        events.join("\n") + "\n"
    }

    #[test]
    fn parses_terminal_text_tool_evidence_and_usage() {
        let parsed = parse_job_jsonl(
            &stream(&[
                r#"{"type":"session","version":3,"id":"s1"}"#,
                r#"{"type":"agent_start"}"#,
                r#"{"type":"tool_execution_end","toolCallId":"t1","toolName":"bash","result":{"text":"ok"},"isError":false}"#,
                r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"done"}],"usage":{"inputTokens":11,"outputTokens":7,"cost":{"total":0.02}}}}"#,
                r#"{"type":"agent_end","messages":[]}"#,
            ]),
            3,
        )
        .unwrap();
        assert_eq!(parsed.final_text, "done");
        assert_eq!(parsed.usage.input_tokens, 11);
        assert_eq!(parsed.usage.output_tokens, 7);
        assert_eq!(parsed.usage.cost_usd, Some(0.02));
        assert_eq!(parsed.evidence.len(), 1);
    }

    #[test]
    fn does_not_double_count_usage_repeated_by_turn_end() {
        let parsed = parse_job_jsonl(
            &stream(&[
                r#"{"type":"session","version":3,"id":"s1"}"#,
                r#"{"type":"agent_start"}"#,
                r#"{"type":"message_end","message":{"role":"assistant","content":"done","usage":{"inputTokens":11,"outputTokens":7}}}"#,
                r#"{"type":"turn_end","message":{"role":"assistant","content":"done","usage":{"inputTokens":11,"outputTokens":7}}}"#,
                r#"{"type":"agent_end","messages":[]}"#,
            ]),
            3,
        )
        .unwrap();
        assert_eq!(parsed.usage.input_tokens, 11);
        assert_eq!(parsed.usage.output_tokens, 7);
    }

    #[test]
    fn rejects_malformed_truncated_drifted_and_privileged_streams() {
        for input in [
            stream(&[
                r#"{"type":"session","version":3,"id":"s1"}"#,
                r#"{"type":"agent_start"}"#,
            ]),
            stream(&[
                r#"{"type":"session","version":4,"id":"s1"}"#,
                r#"{"type":"agent_start"}"#,
                r#"{"type":"agent_end","messages":[]}"#,
            ]),
            stream(&[
                r#"{"type":"session","version":3,"id":"s1"}"#,
                r#"{"type":"agent_start"}"#,
                r#"{"type":"extension_ui_request","id":"x","method":"confirm"}"#,
                r#"{"type":"agent_end","messages":[]}"#,
            ]),
            "not-json\n".into(),
        ] {
            assert!(parse_job_jsonl(&input, 3).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn rpc_commands_use_strict_lf_framing_and_expected_names() {
        assert_eq!(
            PiRpcCommand::FollowUp {
                id: "1".into(),
                message: "later".into()
            }
            .to_jsonl()
            .unwrap(),
            "{\"type\":\"follow_up\",\"id\":\"1\",\"message\":\"later\"}\n"
        );
    }

    #[test]
    fn rpc_parser_correlates_responses_and_rejects_privileged_or_non_lf_records() {
        let command = PiRpcCommand::GetState { id: "s1".into() };
        let record = parse_rpc_record(
            b"{\"type\":\"response\",\"command\":\"get_state\",\"id\":\"s1\",\"success\":true,\"data\":{\"isStreaming\":false}}\n",
        )
        .unwrap();
        assert_eq!(
            validate_rpc_response(record, &command).unwrap().unwrap()["isStreaming"],
            false
        );
        assert!(parse_rpc_record(
            b"{\"type\":\"extension_ui_request\",\"id\":\"x\",\"method\":\"confirm\"}\n"
        )
        .is_err());
        assert!(parse_rpc_record(b"{\"type\":\"agent_start\"}\r\n").is_err());
        assert!(parse_rpc_record(b"{\"type\":\"agent_start\"}").is_err());
    }
}
