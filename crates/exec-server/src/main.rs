use std::io::{self, BufRead, Write};

mod filesystem;
mod process;
mod protocol;

fn main() -> io::Result<()> {
    let expected_secret = std::env::var("ALETHEON_EXEC_SERVER_SECRET").map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "ALETHEON_EXEC_SERVER_SECRET must be set",
        )
    })?;
    if expected_secret.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ALETHEON_EXEC_SERVER_SECRET must not be empty",
        ));
    }
    let workspace_roots = filesystem::WorkspaceRoots::from_env()?;
    let file_manager = filesystem::FileManager::default();
    let stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let process_mgr = process::ProcessManager::new();
    // This stdio transport is one authenticated connection. Every child it
    // launches is tagged with this identity so EOF/error cleanup is scoped.
    let connection_owner = format!("stdio:{}", std::process::id());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");

    let mut handshake_done = false;

    for line in stdin.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let resp = protocol::Response::err(
                    serde_json::Value::Null,
                    protocol::PARSE_ERROR,
                    format!("I/O error reading input: {}", e),
                );
                let _ = write_response(&mut stdout, &resp);
                break;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: protocol::Request = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let resp = protocol::Response::err(
                    serde_json::Value::Null,
                    protocol::PARSE_ERROR,
                    format!("Parse error: {}", e),
                );
                if write_response(&mut stdout, &resp).is_err() {
                    break;
                }
                continue;
            }
        };

        // Require an exact-secret handshake as the first successful message.
        // A rejected attempt never authenticates the connection.
        let handshake_attempted = !handshake_done && request.method == "handshake";
        let response = if !handshake_done {
            if request.method != "handshake" {
                let resp = protocol::Response::err(
                    request.id.clone(),
                    protocol::INVALID_REQUEST,
                    "Handshake required as first message".to_string(),
                );
                if write_response(&mut stdout, &resp).is_err() {
                    break;
                }
                continue;
            }
            let response = handle_handshake(&request, &expected_secret);
            handshake_done = matches!(&response.result, protocol::ResponseResult::Ok { .. });
            response
        } else {
            dispatch(
                &request,
                &process_mgr,
                &rt,
                &workspace_roots,
                &file_manager,
                &connection_owner,
            )
        };

        if write_response(&mut stdout, &response).is_err() {
            break;
        }
        // Authentication is single-shot. A rejected secret/handshake payload
        // closes the transport instead of allowing online guessing.
        if handshake_attempted && !handshake_done {
            break;
        }

        // Shutdown command exits the loop
        if request.method == "shutdown" {
            break;
        }
    }

    // EOF, input failure, or shutdown closes this authenticated connection.
    // Deterministically terminate and reap only its foreground children.
    rt.block_on(process_mgr.cleanup_owner(&connection_owner));
    rt.block_on(process_mgr.shutdown());

    Ok(())
}

fn write_response(writer: &mut impl Write, response: &protocol::Response) -> io::Result<()> {
    let json = serde_json::to_string(response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    writeln!(writer, "{json}")?;
    writer.flush()
}

fn dispatch(
    req: &protocol::Request,
    pm: &process::ProcessManager,
    rt: &tokio::runtime::Runtime,
    workspace_roots: &filesystem::WorkspaceRoots,
    file_manager: &filesystem::FileManager,
    connection_owner: &str,
) -> protocol::Response {
    match req.method.as_str() {
        "ping" => protocol::Response::ok(req.id.clone(), serde_json::json!({"status": "ok"})),
        "process/start" => handle_process_start(req, pm, rt, connection_owner),
        "process/read" => handle_process_read(req, pm, rt),
        "process/write" => handle_process_write(req, pm, rt),
        "process/signal" => handle_process_signal(req, pm, rt),
        "process/terminate" => handle_process_terminate(req, pm, rt),
        "process/kill" => handle_process_terminate(req, pm, rt),
        "shutdown" => protocol::Response::ok(
            req.id.clone(),
            serde_json::json!({"status": "shutting_down"}),
        ),
        // Try filesystem methods
        method if method.starts_with("fs/") => {
            filesystem::handle_fs(method, &req.params, workspace_roots, file_manager)
                .map(|mut r| {
                    r.id = req.id.clone();
                    r
                })
                .unwrap_or_else(|| {
                    protocol::Response::err(
                        req.id.clone(),
                        protocol::METHOD_NOT_FOUND,
                        format!("Method not found: {}", req.method),
                    )
                })
        }
        _ => protocol::Response::err(
            req.id.clone(),
            protocol::METHOD_NOT_FOUND,
            format!("Method not found: {}", req.method),
        ),
    }
}

fn handle_handshake(req: &protocol::Request, expected_secret: &str) -> protocol::Response {
    let hs_req: protocol::HandshakeRequest = match serde_json::from_value(req.params.clone()) {
        Ok(h) => h,
        Err(e) => {
            return protocol::Response::err(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                format!("Invalid handshake params: {}", e),
            );
        }
    };

    if hs_req.secret != expected_secret {
        return protocol::Response::err(
            req.id.clone(),
            protocol::UNAUTHORIZED,
            "Handshake rejected".to_string(),
        );
    }

    let server_pid = std::process::id();

    let hs_resp = protocol::HandshakeResponse {
        protocol_version: 1,
        server_pid,
    };

    protocol::Response::ok(
        req.id.clone(),
        serde_json::to_value(hs_resp).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handshake(secret: &str) -> protocol::Request {
        protocol::Request {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(1),
            method: "handshake".into(),
            params: serde_json::json!({"secret": secret}),
        }
    }

    #[test]
    fn handshake_requires_exact_pre_shared_secret() {
        assert!(matches!(
            handle_handshake(&handshake("expected"), "expected").result,
            protocol::ResponseResult::Ok { .. }
        ));
        assert!(matches!(
            handle_handshake(&handshake("wrong"), "expected").result,
            protocol::ResponseResult::Err { error }
                if error.code == protocol::UNAUTHORIZED
        ));
        assert!(matches!(
            handle_handshake(&handshake(""), "expected").result,
            protocol::ResponseResult::Err { error }
                if error.code == protocol::UNAUTHORIZED
        ));
    }
}

fn handle_process_start(
    req: &protocol::Request,
    pm: &process::ProcessManager,
    rt: &tokio::runtime::Runtime,
    connection_owner: &str,
) -> protocol::Response {
    let start_req: protocol::StartProcessRequest = match serde_json::from_value(req.params.clone())
    {
        Ok(s) => s,
        Err(e) => {
            return protocol::Response::err(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                format!("Invalid process/start params: {}", e),
            );
        }
    };

    match rt.block_on(pm.spawn(connection_owner, &start_req)) {
        Ok(handle) => protocol::Response::ok(
            req.id.clone(),
            serde_json::to_value(handle).unwrap_or_default(),
        ),
        Err(rpc_err) => protocol::Response::err(req.id.clone(), rpc_err.code, rpc_err.message),
    }
}

fn handle_process_read(
    req: &protocol::Request,
    pm: &process::ProcessManager,
    rt: &tokio::runtime::Runtime,
) -> protocol::Response {
    let handle_id: String = match req.params.get("handle_id").and_then(|v| v.as_str()) {
        Some(h) => h.to_string(),
        None => {
            return protocol::Response::err(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing 'handle_id' parameter".to_string(),
            );
        }
    };

    match rt.block_on(pm.read(&handle_id)) {
        Ok(chunks) => protocol::Response::ok(
            req.id.clone(),
            serde_json::to_value(chunks).unwrap_or_default(),
        ),
        Err(rpc_err) => protocol::Response::err(req.id.clone(), rpc_err.code, rpc_err.message),
    }
}

fn handle_process_write(
    req: &protocol::Request,
    pm: &process::ProcessManager,
    rt: &tokio::runtime::Runtime,
) -> protocol::Response {
    let handle_id: String = match req.params.get("handle_id").and_then(|v| v.as_str()) {
        Some(h) => h.to_string(),
        None => {
            return protocol::Response::err(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing 'handle_id' parameter".to_string(),
            );
        }
    };

    let data: String = match req.params.get("data").and_then(|v| v.as_str()) {
        Some(d) => d.to_string(),
        None => {
            return protocol::Response::err(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing 'data' parameter".to_string(),
            );
        }
    };

    match rt.block_on(pm.write_stdin(&handle_id, &data)) {
        Ok(()) => protocol::Response::ok(req.id.clone(), serde_json::json!({"status": "ok"})),
        Err(rpc_err) => protocol::Response::err(req.id.clone(), rpc_err.code, rpc_err.message),
    }
}

fn handle_process_signal(
    req: &protocol::Request,
    pm: &process::ProcessManager,
    rt: &tokio::runtime::Runtime,
) -> protocol::Response {
    let handle_id: String = match req.params.get("handle_id").and_then(|v| v.as_str()) {
        Some(h) => h.to_string(),
        None => {
            return protocol::Response::err(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing 'handle_id' parameter".to_string(),
            );
        }
    };

    let sig: i32 = match req.params.get("signal").and_then(|v| v.as_i64()) {
        Some(s) => s as i32,
        None => {
            return protocol::Response::err(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing 'signal' parameter".to_string(),
            );
        }
    };

    match rt.block_on(pm.signal(&handle_id, sig)) {
        Ok(()) => protocol::Response::ok(req.id.clone(), serde_json::json!({"status": "ok"})),
        Err(rpc_err) => protocol::Response::err(req.id.clone(), rpc_err.code, rpc_err.message),
    }
}

fn handle_process_terminate(
    req: &protocol::Request,
    pm: &process::ProcessManager,
    rt: &tokio::runtime::Runtime,
) -> protocol::Response {
    let handle_id: String = match req.params.get("handle_id").and_then(|v| v.as_str()) {
        Some(h) => h.to_string(),
        None => {
            return protocol::Response::err(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing 'handle_id' parameter".to_string(),
            );
        }
    };

    match rt.block_on(pm.terminate(&handle_id)) {
        Ok(()) => {
            protocol::Response::ok(req.id.clone(), serde_json::json!({"status": "terminated"}))
        }
        Err(rpc_err) => protocol::Response::err(req.id.clone(), rpc_err.code, rpc_err.message),
    }
}
