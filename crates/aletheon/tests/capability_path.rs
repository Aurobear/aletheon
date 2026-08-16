use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

// Source modules keep their cfg(test) module at the end. Architecture scans
// intentionally inspect only the production prefix; raw contract calls remain
// useful in unit tests but must never become a shipping execution path.
fn production_source(path: &Path) -> String {
    let source = fs::read_to_string(path).unwrap();
    source
        .split("#[cfg(test)]")
        .next()
        .unwrap_or(&source)
        .to_string()
}

#[test]
fn production_has_one_governed_capability_construction() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);

    let mut default_constructors = Vec::new();
    for path in files {
        let source = production_source(&path);
        if source.contains("DefaultCapabilityInvoker::new") {
            default_constructors.push(path.clone());
        }
        // Agent-control owns a separate admission and settlement lifecycle;
        // its terminology must not be mistaken for capability admission.
        let is_canonical_boundary =
            path.ends_with("src/wiring/wiring/application/governed_capability.rs");
        // RA-04's RuntimeTurnWriter is a separate canonical lifecycle writer;
        // its terminal fence is intentionally not the Corpus capability
        // settlement path this census protects.
        let is_runtime_turn_writer_boundary =
            source.contains("runtime_turn_writer") && source.contains("runtime::TurnTerminal");
        if !is_canonical_boundary
            && !path
                .components()
                .any(|part| part.as_os_str() == "agent_control")
        {
            for forbidden in [".admit(", ".settle(", "AdmissionRequest {"] {
                if forbidden == ".settle(" && is_runtime_turn_writer_boundary {
                    continue;
                }
                assert!(
                    !source.contains(forbidden),
                    "{} bypasses the governed capability lifecycle with {forbidden}",
                    path.display()
                );
            }
        }
        assert!(
            !source.contains("tool.execute("),
            "{} executes a raw Tool outside Corpus runtime",
            path.display()
        );
    }

    // The governed capability invoker now lives in Kernel (capability owner).
    // `canonical_capability_invoker` is the single authoritative constructor;
    // the binary-owned wiring factory reuses it rather than minting a second.
    assert!(
        default_constructors.is_empty(),
        "aletheon must not construct the invoker directly"
    );
    let kernel_root = root.join("../kernel/src");
    let mut kernel_files = Vec::new();
    rust_files(&kernel_root, &mut kernel_files);
    let canonical = kernel_files
        .iter()
        .filter(|path| production_source(path).contains("pub fn canonical_capability_invoker"))
        .count();
    assert_eq!(canonical, 1, "one canonical invoker constructor in kernel");
}

#[test]
fn external_and_provider_paths_use_the_capability_service() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let provider = production_source(&root.join("src/wiring/adapters/runtime/provider_worker.rs"));
    let mcp = production_source(&root.join("../aletheon/src/wiring/daemon/mcp_embedded.rs"));
    // The configured-agent execution path moved with the daemon composition
    // owner. Keep this architecture assertion aimed at the shipping path
    // rather than a retired Executive source locator.
    let configured = production_source(&root.join("../aletheon/src/wiring/exec_session.rs"));

    for (name, source) in [
        ("provider worker", provider),
        ("MCP", mcp),
        ("configured agent", configured),
    ] {
        assert!(
            source.contains(".capability") || source.contains("capability.invoke"),
            "{name} is not wired through CapabilityService"
        );
        assert!(
            !source.contains("tool.execute("),
            "{name} executes raw tools"
        );
    }
}
