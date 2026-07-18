//! JSON-RPC 2.0 protocol types for exec-server.

use serde::{Deserialize, Serialize};

/// JSON-RPC 2.0 request.
#[derive(Debug, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(flatten)]
    pub result: ResponseResult,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ResponseResult {
    Ok { result: serde_json::Value },
    Err { error: RpcError },
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// Standard JSON-RPC error codes
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

// Exec-server specific error codes
pub const SPAWN_FAILED: i32 = -32000;
pub const PROCESS_NOT_FOUND: i32 = -32001;
pub const FS_ACCESS_DENIED: i32 = -32002;
pub const BUFFER_OVERFLOW: i32 = -32003;
pub const TIMEOUT: i32 = -32004;
pub const UNAUTHORIZED: i32 = -32005;

/// Handshake request from daemon.
#[derive(Debug, Deserialize)]
pub struct HandshakeRequest {
    pub secret: String,
}

/// Handshake response.
#[derive(Debug, Serialize)]
pub struct HandshakeResponse {
    pub protocol_version: u32,
    pub server_pid: u32,
}

/// Process start parameters.
#[derive(Debug, Deserialize)]
pub struct StartProcessRequest {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_output_bytes: Option<u64>,
}

/// Process handle returned on successful start.
#[derive(Debug, Serialize)]
pub struct ProcessHandle {
    pub pid: u32,
    pub handle_id: String,
}

/// Read chunk from process output.
#[derive(Debug, Serialize)]
pub struct ReadChunk {
    pub data: String,
    pub stream: String, // "stdout" or "stderr"
    pub eof: bool,
}

impl Response {
    pub fn ok(id: serde_json::Value, result: serde_json::Value) -> Self {
        Response {
            jsonrpc: "2.0".into(),
            id,
            result: ResponseResult::Ok { result },
        }
    }

    pub fn err(id: serde_json::Value, code: i32, message: String) -> Self {
        Response {
            jsonrpc: "2.0".into(),
            id,
            result: ResponseResult::Err {
                error: RpcError {
                    code,
                    message,
                    data: None,
                },
            },
        }
    }
}
