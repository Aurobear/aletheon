//! Agent communication trait and JSON-RPC message types.
//!
//! Provides an async trait for inter-agent messaging over Unix sockets
//! using a JSON-RPC inspired protocol.
//!
//! ## Trait implementation status
//!
//! `AgentCommunication` is a **future design placeholder** — the
//! JSON-RPC request/response types are used in tests and serve as a
//! concrete protocol contract, but the trait itself has no
//! implementations yet. It will be wired to a real transport
//! (Unix-domain sockets or the mailbox bus) when inter-agent
//! messaging is implemented.

use super::{AgentId, Endpoint};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A JSON-RPC style request sent between agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Method name.
    pub method: String,
    /// Parameters as a JSON value.
    pub params: serde_json::Value,
    /// Optional request ID for correlating responses.
    pub id: Option<u64>,
}

impl JsonRpcRequest {
    /// Create a new request.
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
            id: None,
        }
    }

    /// Create a new request with a correlation ID.
    pub fn with_id(method: impl Into<String>, params: serde_json::Value, id: u64) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
            id: Some(id),
        }
    }
}

/// A JSON-RPC style response received from an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Successful result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    /// Correlation ID matching the request.
    pub id: Option<u64>,
}

impl JsonRpcResponse {
    /// Create a success response.
    pub fn success(result: serde_json::Value, id: Option<u64>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Create an error response.
    pub fn error(code: i64, message: impl Into<String>, id: Option<u64>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
            id,
        }
    }

    /// Returns true if this response indicates success.
    pub fn is_success(&self) -> bool {
        self.error.is_none() && self.result.is_some()
    }
}

/// JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code.
    pub code: i64,
    /// Error message.
    pub message: String,
    /// Optional additional error data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Async trait for inter-agent communication.
///
/// Implementations handle the transport layer (Unix sockets, TCP, etc.)
/// while callers use this trait for protocol-level interaction.
#[deprecated(note = "No implementations exist — design placeholder, not yet wired to a transport")]
#[async_trait]
pub trait AgentCommunication: Send + Sync {
    /// Send a message to a specific agent.
    ///
    /// Returns the response from the target agent.
    async fn send_message(
        &self,
        target: &AgentId,
        endpoint: &Endpoint,
        request: JsonRpcRequest,
    ) -> Result<JsonRpcResponse>;

    /// Broadcast a message to all known agents.
    ///
    /// Returns responses from agents that replied (some may time out).
    async fn broadcast(
        &self,
        endpoints: &[(AgentId, Endpoint)],
        request: JsonRpcRequest,
    ) -> Vec<(AgentId, Result<JsonRpcResponse>)>;

    /// Send a request and wait for a response with timeout.
    ///
    /// Convenience wrapper around `send_message` with timeout handling.
    async fn request(
        &self,
        target: &AgentId,
        endpoint: &Endpoint,
        method: impl Into<String> + Send,
        params: serde_json::Value,
    ) -> Result<JsonRpcResponse> {
        let request = JsonRpcRequest::new(method, params);
        self.send_message(target, endpoint, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_rpc_request_serialization() {
        let req = JsonRpcRequest::new("agent.status", serde_json::json!({"agent_id": "test"}));
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"agent.status\""));
        assert!(json.contains("\"id\":null") || !json.contains("\"id\""));
    }

    #[test]
    fn test_json_rpc_request_with_id() {
        let req = JsonRpcRequest::with_id("agent.ping", serde_json::json!({}), 42);
        assert_eq!(req.id, Some(42));
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("42"));
    }

    #[test]
    fn test_json_rpc_response_success() {
        let resp = JsonRpcResponse::success(serde_json::json!({"status": "ok"}), Some(1));
        assert!(resp.is_success());
        assert_eq!(resp.id, Some(1));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_json_rpc_response_error() {
        let resp = JsonRpcResponse::error(-32600, "Invalid Request", Some(2));
        assert!(!resp.is_success());
        assert!(resp.result.is_none());
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, -32600);
        assert_eq!(err.message, "Invalid Request");
    }

    #[test]
    fn test_json_rpc_response_roundtrip() {
        let resp = JsonRpcResponse::success(serde_json::json!({"agents": []}), Some(99));
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: JsonRpcResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, Some(99));
        assert!(deserialized.is_success());
    }
}
