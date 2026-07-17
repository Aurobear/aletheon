use std::io::{self, BufRead, Write};

mod protocol;
mod process;
mod filesystem;

fn main() -> io::Result<()> {
    let stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut process_mgr = process::ProcessManager::new();
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
                writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                stdout.flush()?;
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
                writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                stdout.flush()?;
                continue;
            }
        };

        // Require handshake as first message
        if !handshake_done {
            if request.method != "handshake" {
                let resp = protocol::Response::err(
                    request.id.clone(),
                    protocol::INVALID_REQUEST,
                    "Handshake required as first message".to_string(),
                );
                writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                stdout.flush()?;
                continue;
            }
            handshake_done = true;
        }

        let response = dispatch(&request, &mut process_mgr, &rt);

        writeln!(stdout, "{}", match serde_json::to_string(&response) {
            Ok(json) => json,
            Err(e) => {
                let fallback = protocol::Response::err(
                    request.id.clone(),
                    protocol::INTERNAL_ERROR,
                    format!("Failed to serialize response: {}", e),
                );
                serde_json::to_string(&fallback).unwrap_or_else(|_| {
                    r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error"}}"#.to_string()
                })
            }
        })?;
        stdout.flush()?;

        // Shutdown command exits the loop
        if request.method == "shutdown" {
            break;
        }
    }

    // Kill remaining processes on exit
    rt.block_on(process_mgr.shutdown());

    Ok(())
}

fn dispatch(
    req: &protocol::Request,
    pm: &process::ProcessManager,
    rt: &tokio::runtime::Runtime,
) -> protocol::Response {
    match req.method.as_str() {
        "handshake" => handle_handshake(req),
        "process/start" => handle_process_start(req, pm, rt),
        "process/read" => handle_process_read(req, pm, rt),
        "process/write" => handle_process_write(req, pm, rt),
        "process/signal" => handle_process_signal(req, pm, rt),
        "process/terminate" => handle_process_terminate(req, pm, rt),
        "shutdown" => protocol::Response::ok(
            req.id.clone(),
            serde_json::json!({"status": "shutting_down"}),
        ),
        // Try filesystem methods
        method if method.starts_with("fs/") => {
            filesystem::handle_fs(method, &req.params).map(|mut r| {
                r.id = req.id.clone();
                r
            }).unwrap_or_else(|| protocol::Response::err(
                req.id.clone(),
                protocol::METHOD_NOT_FOUND,
                format!("Method not found: {}", req.method),
            ))
        }
        _ => protocol::Response::err(
            req.id.clone(),
            protocol::METHOD_NOT_FOUND,
            format!("Method not found: {}", req.method),
        ),
    }
}

fn handle_handshake(req: &protocol::Request) -> protocol::Response {
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

    // Accept any non-empty secret (real validation in follow-up)
    if hs_req.secret.is_empty() {
        return protocol::Response::err(
            req.id.clone(),
            protocol::INVALID_PARAMS,
            "Handshake secret must not be empty".to_string(),
        );
    }

    let server_pid = std::process::id();

    let hs_resp = protocol::HandshakeResponse {
        protocol_version: 1,
        server_pid,
    };

    protocol::Response::ok(req.id.clone(), serde_json::to_value(hs_resp).unwrap_or_default())
}

fn handle_process_start(
    req: &protocol::Request,
    pm: &process::ProcessManager,
    rt: &tokio::runtime::Runtime,
) -> protocol::Response {
    let start_req: protocol::StartProcessRequest = match serde_json::from_value(req.params.clone()) {
        Ok(s) => s,
        Err(e) => {
            return protocol::Response::err(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                format!("Invalid process/start params: {}", e),
            );
        }
    };

    match rt.block_on(pm.spawn(&start_req)) {
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
        Ok(()) => protocol::Response::ok(
            req.id.clone(),
            serde_json::json!({"status": "ok"}),
        ),
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
        Ok(()) => protocol::Response::ok(
            req.id.clone(),
            serde_json::json!({"status": "ok"}),
        ),
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
        Ok(()) => protocol::Response::ok(
            req.id.clone(),
            serde_json::json!({"status": "terminated"}),
        ),
        Err(rpc_err) => protocol::Response::err(req.id.clone(), rpc_err.code, rpc_err.message),
    }
}
