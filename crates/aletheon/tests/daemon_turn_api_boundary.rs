use std::fs;

fn source(path: &str) -> String {
    fs::read_to_string(path).expect("production source")
}

#[test]
fn daemon_turn_composition_resources_do_not_escape_the_crate() {
    let orchestrator = source("src/wiring/application/daemon_turn/orchestrator.rs");
    assert!(orchestrator.contains("pub(crate) struct DaemonTurnResources"));
    assert!(orchestrator.contains("pub(crate) fn new(resources: DaemonTurnResources)"));
    assert!(!orchestrator.contains("pub fn new(resources: DaemonTurnResources)"));

    let module = source("src/wiring/application/daemon_turn/mod.rs");
    assert!(!module.contains("use orchestrator::DaemonTurnResources"));
    assert!(!module.contains("pub use orchestrator::DaemonTurnResources"));

    for path in ["src/wiring/application/mod.rs", "src/lib.rs"] {
        let public_surface = source(path);
        for private in [
            "DaemonTurnResources",
            "TurnPipelineResources",
            "DaemonTurnTestBuilder",
            "DaemonTurnTestHarness",
            "TestAletheonBuilder",
        ] {
            assert!(
                !public_surface.contains(private),
                "{path} leaked test/composition API: {private}"
            );
        }
    }
}

#[test]
fn any_in_crate_test_support_module_is_test_cfg_gated() {
    for path in ["src/lib.rs", "src/wiring/application/mod.rs"] {
        let source = source(path);
        if let Some(module_offset) = source.find("mod test_support") {
            let prefix = &source[..module_offset];
            let declaration_window = &prefix[prefix.len().saturating_sub(160)..];
            assert!(
                declaration_window.contains("#[cfg(test)]"),
                "{path} test_support must be guarded by #[cfg(test)]"
            );
            let declaration_line = source[module_offset..].lines().next().unwrap_or_default();
            assert!(
                !declaration_line.starts_with("pub mod test_support"),
                "{path} must not publicly export test_support"
            );
        }
    }
}

#[test]
fn daemon_turn_resources_are_constructed_only_in_bootstrap() {
    let mut violations = Vec::new();
    for root in [
        "src/application",
        "src/core",
        "../aletheon/src/wiring/daemon/bootstrap",
    ] {
        let root = std::path::PathBuf::from(root);
        if !root.exists() {
            continue;
        }
        let mut stack = vec![root];
        while let Some(directory) = stack.pop() {
            for entry in fs::read_dir(directory).expect("source directory") {
                let path = entry.expect("source entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs")
                    && source(path.to_str().unwrap()).contains("DaemonTurnResources {")
                    && !path.ends_with("daemon_turn/orchestrator.rs")
                    && !path.starts_with("../aletheon/src/wiring/daemon/bootstrap")
                {
                    violations.push(path);
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "resource construction escaped bootstrap: {violations:?}"
    );
}

#[test]
fn production_has_one_turn_engine_interface_and_no_permission_fallback_adapter() {
    let mut implementations = Vec::new();
    let mut bindings = Vec::new();
    let mut stack = vec![std::path::PathBuf::from("src")];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(directory).expect("source directory") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let code = source(path.to_str().unwrap());
                if code.contains("impl TurnEngine for") {
                    implementations.push(path.clone());
                }
                if code.contains("DaemonTurnEngine::new(") {
                    bindings.push(path.clone());
                }
            }
        }
    }

    let engine_boundary = format!(
        "{}\n{}",
        source("src/wiring/application/turn_engine.rs"),
        source("src/wiring/application/daemon_turn_engine.rs")
    );
    for forbidden in [
        "SessionTurnEngine",
        "LocalOsPrincipal { uid: 0, gid: 0 }",
        "PermissionProfileId(\"exec\"",
        "ApprovalPolicy::Never",
    ] {
        assert!(
            !engine_boundary.contains(forbidden),
            "TurnEngine boundary retains forbidden R1 fallback {forbidden}"
        );
    }

    implementations.sort();
    assert_eq!(
        implementations,
        [
            std::path::PathBuf::from("src/wiring/application/daemon_turn_engine.rs"),
            std::path::PathBuf::from("src/wiring/exec_session.rs"),
        ],
        "daemon and exec must both enter the TurnEngine interface"
    );
    assert_eq!(
        bindings,
        [std::path::PathBuf::from(
            "src/wiring/application/daemon_turn/orchestrator.rs"
        )],
        "production must have one TurnEngine binding point"
    );
}

#[test]
fn daemon_turn_scope_is_local_and_never_stored_in_a_shared_option() {
    let pipeline = source("src/wiring/application/turn_pipeline.rs");
    let engine = source("src/wiring/application/daemon_turn_engine.rs");
    let bootstrap = source("../aletheon/src/wiring/daemon/bootstrap/turn_runtime.rs");
    let production = format!("{pipeline}\n{engine}\n{bootstrap}");

    assert!(!production.contains("current_scope"));
    assert!(!production.contains("Option<OperationScope>"));
    assert!(engine.contains("OperationScope::with_cancellation("));
    assert!(engine.contains("&mut scope"));
    assert!(engine.contains("settle_and_drain("));
    assert!(engine.contains("abort_and_drain("));
}

#[test]
fn turn_result_crosses_the_pipeline_boundary_as_a_typed_outcome_not_json() {
    let engine = source("src/wiring/application/daemon_turn_engine.rs");
    let pipeline = source("src/wiring/application/turn_pipeline.rs");

    // R3: the daemon engine must consume the typed TurnPipelineOutcome directly
    // and must not reverse-index a serialized turn envelope.
    assert!(
        engine.contains("TurnPipelineOutcome::Completed(execution)"),
        "daemon engine must destructure the typed pipeline outcome"
    );
    assert!(
        !engine.contains("response.get(\"error\")"),
        "daemon engine must not probe a JSON error envelope"
    );
    assert!(
        !engine.contains("serde_json::from_value::<::contracts::TurnResult>"),
        "daemon engine must not re-parse turn_result JSON"
    );
    assert!(
        !engine.contains("raw[\"turn_result\"]"),
        "daemon engine must not index turn_result JSON"
    );

    // The pipeline returns the typed outcome and owns TurnExecution construction.
    assert!(
        pipeline.contains("pub enum TurnPipelineOutcome"),
        "pipeline must expose the typed outcome"
    );
    assert!(
        pipeline.contains("Completed(Box<TurnExecution>)"),
        "pipeline outcome must carry the typed TurnExecution"
    );
}
