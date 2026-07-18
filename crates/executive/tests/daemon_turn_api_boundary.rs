use std::fs;

fn source(path: &str) -> String {
    fs::read_to_string(path).expect("production source")
}

#[test]
fn daemon_turn_composition_resources_do_not_escape_the_crate() {
    let orchestrator = source("src/service/daemon_turn/orchestrator.rs");
    assert!(orchestrator.contains("pub(crate) struct DaemonTurnResources"));
    assert!(orchestrator.contains("pub(crate) fn new(resources: DaemonTurnResources)"));
    assert!(!orchestrator.contains("pub fn new(resources: DaemonTurnResources)"));

    let module = source("src/service/daemon_turn/mod.rs");
    assert!(module.contains("pub(crate) use orchestrator::DaemonTurnResources"));
    assert!(!module.contains("pub use orchestrator::DaemonTurnResources"));

    for path in ["src/service/mod.rs", "src/lib.rs"] {
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
    for path in ["src/lib.rs", "src/service/mod.rs"] {
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
    for root in ["src/service", "src/core", "src/host"] {
        let mut stack = vec![std::path::PathBuf::from(root)];
        while let Some(directory) = stack.pop() {
            for entry in fs::read_dir(directory).expect("source directory") {
                let path = entry.expect("source entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs")
                    && source(path.to_str().unwrap()).contains("DaemonTurnResources {")
                    && !path.ends_with("daemon_turn/orchestrator.rs")
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
