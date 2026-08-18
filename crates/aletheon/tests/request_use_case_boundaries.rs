use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).expect("read source directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn request_handler_owns_only_ports_and_protocol_state() {
    let source =
        fs::read_to_string("../aletheon/src/daemon/handler/mod.rs").expect("handler source");
    let start = source
        .find("pub struct RequestHandler")
        .expect("RequestHandler");
    let body = &source[start
        ..source[start..]
            .find("}\n\nimpl RequestHandler")
            .map(|end| start + end + 1)
            .expect("struct end")];
    for forbidden in [
        "CoreSystems",
        "subsystems",
        "MemoryGroup",
        "CorpusGroup",
        "SecurityGroup",
    ] {
        assert!(
            !body.contains(forbidden),
            "RequestHandler contains forbidden dependency {forbidden}"
        );
    }
    assert!(body.contains("ports: Arc<ports::HandlerPorts>"));
}

#[test]
fn rpc_adapters_do_not_touch_domain_implementation_details() {
    let root = Path::new("../aletheon/src/daemon/handler/rpc");
    let forbidden = [
        "subsystems",
        "CoreSystems",
        "MemoryGroup",
        "CorpusGroup",
        "SecurityGroup",
        "FactStore",
        "ObjectiveStore",
        "ApprovalRepository",
        ".lock()",
    ];
    let mut violations = Vec::new();
    for file in rust_files(root) {
        let source = fs::read_to_string(&file).expect("rpc source");
        for needle in forbidden {
            if source.contains(needle) {
                violations.push(format!("{}: {needle}", file.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "request-boundary violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn handler_ports_cover_every_rpc_family() {
    let source =
        fs::read_to_string("../aletheon/src/daemon/handler/ports.rs").expect("ports source");
    for field in [
        "facts",
        "goals",
        "approvals",
        "admin",
        "sessions",
        "health",
        "google",
        "workflow",
        "turn",
    ] {
        assert!(
            source.contains(&format!("pub(crate) {field}:")),
            "missing HandlerPorts field {field}"
        );
    }
}

#[test]
fn typed_prompt_target_reaches_the_targeted_turn_use_case() {
    let handler =
        fs::read_to_string("../aletheon/src/daemon/handler/mod.rs").expect("handler source");
    let submit_start = handler
        .find("impl CommandUseCases for DaemonCommandUseCases")
        .expect("daemon command use cases");
    let submit_end = handler[submit_start..]
        .find("async fn execute_shell")
        .map(|offset| submit_start + offset)
        .expect("submit_prompt end");
    let submit_prompt = &handler[submit_start..submit_end];
    assert!(
        submit_prompt.contains("prompt.execution_target.clone()"),
        "typed ClientIntent target was dropped before the explicit chat boundary"
    );

    let use_cases =
        fs::read_to_string("src/host/request_use_cases.rs").expect("request use case source");
    assert!(
        use_cases.contains("execution_target: ::contracts::ExecutionTargetSelection"),
        "TurnUseCases no longer carries the typed target"
    );
    let turn_adapter = fs::read_to_string("src/daemon/turn_use_cases.rs")
        .expect("daemon turn use case adapter source");
    assert!(
        turn_adapter.contains(".execute_turn_targeted("),
        "production TurnUseCases silently restored the default General path"
    );
}
