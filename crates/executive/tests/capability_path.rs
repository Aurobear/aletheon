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
        for forbidden in [".admit(", ".settle(", "AdmissionRequest {"] {
            assert!(
                !source.contains(forbidden),
                "{} bypasses the governed capability lifecycle with {forbidden}",
                path.display()
            );
        }
        assert!(
            !source.contains("tool.execute("),
            "{} executes a raw Tool outside Corpus runtime",
            path.display()
        );
    }

    assert_eq!(default_constructors.len(), 1, "one canonical inner invoker");
    assert!(default_constructors[0].ends_with("service/governed_capability.rs"));
}

#[test]
fn external_and_provider_paths_use_the_capability_service() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let provider = production_source(&root.join("src/impl/runtime/provider_worker.rs"));
    let mcp = production_source(&root.join("src/impl/daemon/mcp_embedded.rs"));
    let configured = production_source(&root.join("src/impl/orchestration/config_agent.rs"));

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
