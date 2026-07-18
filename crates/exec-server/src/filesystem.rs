//! Sandboxed filesystem operations for exec-server.
//!
//! Stub implementations — sandbox profile enforcement comes in a follow-up.
//! All methods return METHOD_NOT_FOUND for now.

use crate::protocol::*;

/// Handle an fs/* RPC method. Returns None if the method is not filesystem-related.
pub fn handle_fs(method: &str, _params: &serde_json::Value) -> Option<Response> {
    let result = match method {
        "fs/read" | "fs/write" | "fs/list" | "fs/metadata" | "fs/remove" => Response::err(
            serde_json::Value::Null,
            METHOD_NOT_FOUND,
            format!("Filesystem operations not yet implemented: {}", method),
        ),
        _ => return None,
    };
    Some(result)
}
