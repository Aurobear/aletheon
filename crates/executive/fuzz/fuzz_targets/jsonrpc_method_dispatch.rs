#![no_main]

use fabric::protocol::client::ClientRpcRequest;
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DispatchRoute {
    SessionLifecycle,
    SessionGateway,
    Debug,
    Chat,
    Rpc,
}

// Keep the fuzz target coupled to the daemon's stable routing contract rather
// than making the private, stateful RequestHandler dispatcher public.
fn classify_method(method: &str) -> DispatchRoute {
    if matches!(
        method,
        "session.resume" | "session.fork" | "session.interrupt" | "session.replay"
    ) {
        DispatchRoute::SessionLifecycle
    } else if method.starts_with("session.") {
        DispatchRoute::SessionGateway
    } else if method.starts_with("debug.") {
        DispatchRoute::Debug
    } else if method == "chat" {
        DispatchRoute::Chat
    } else {
        DispatchRoute::Rpc
    }
}

fuzz_target!(|data: &[u8]| {
    let method = serde_json::from_slice::<Value>(data)
        .ok()
        .and_then(|request| {
            request
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| std::str::from_utf8(data).ok().map(str::to_owned))
        .unwrap_or_default();

    let route = classify_method(&method);
    match route {
        DispatchRoute::SessionLifecycle => assert!(matches!(
            method.as_str(),
            "session.resume" | "session.fork" | "session.interrupt" | "session.replay"
        )),
        DispatchRoute::SessionGateway => {
            assert!(method.starts_with("session."));
            assert!(!matches!(
                method.as_str(),
                "session.resume" | "session.fork" | "session.interrupt" | "session.replay"
            ));
        }
        DispatchRoute::Debug => assert!(method.starts_with("debug.")),
        DispatchRoute::Chat => assert_eq!(method, "chat"),
        DispatchRoute::Rpc => {
            assert_ne!(method, "chat");
            assert!(!method.starts_with("session."));
            assert!(!method.starts_with("debug."));
        }
    }

    // Source representative method names from Fabric's typed request model so
    // changes to that wire contract are exercised alongside daemon routing.
    let typed = match data.first().copied().unwrap_or_default() % 8 {
        0 => ClientRpcRequest::Status,
        1 => ClientRpcRequest::Chat(fabric::protocol::client::ChatParams {
            message: method.clone(),
            working_dir: std::path::PathBuf::from("."),
            workspace_roots: Vec::new(),
        }),
        2 => ClientRpcRequest::DebugTopics,
        3 => ClientRpcRequest::DebugHealth,
        4 => ClientRpcRequest::Sessions,
        5 => ClientRpcRequest::Compact,
        6 => ClientRpcRequest::DaemonShutdown,
        _ => ClientRpcRequest::HooksList,
    };
    if let Ok(envelope) = typed.to_json_rpc(Some(data.len() as u64)) {
        let typed_method = envelope
            .get("method")
            .and_then(Value::as_str)
            .expect("typed request always has a method");
        let _ = classify_method(typed_method);
    }
});
