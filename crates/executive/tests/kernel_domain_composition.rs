use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn kernel_and_domain_composition_are_separate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let kernel_lib = fs::read_to_string(root.join("kernel/src/lib.rs")).unwrap();
    let kernel_runtime = fs::read_to_string(root.join("kernel/src/runtime.rs")).unwrap();
    let core_systems = fs::read_to_string(root.join("executive/src/core/core_systems.rs")).unwrap();
    let domain_ports = fs::read_to_string(root.join("executive/src/core/domain_ports.rs")).unwrap();

    assert!(!kernel_lib.contains("pub mod service"));
    assert!(!kernel_runtime.contains("Agora"));
    assert!(core_systems.contains("pub kernel: Arc<KernelRuntime>"));
    assert!(core_systems.contains("pub domains: crate::core::DomainPorts"));
    assert!(domain_ports.contains("agora: Arc<dyn AgoraOps>"));
}

#[test]
fn production_lifecycle_mutation_has_no_table_escape_hatch() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        for forbidden in [
            "aletheon_kernel::process::ProcessTable",
            "aletheon_kernel::operation::OperationTable",
            "aletheon_kernel::space::InMemorySpaceManager",
            "ServicePorts",
            "fabric::agent::Pid",
            "OperationKind::Other",
            "struct AgentProcess",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} contains forbidden lifecycle authority {forbidden}",
                path.display()
            );
        }
    }
}

#[test]
fn domain_production_code_does_not_construct_kernel_clocks() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for domain in ["cognit", "dasein", "agora"] {
        let mut files = Vec::new();
        rust_files(&crates.join(domain).join("src"), &mut files);
        for path in files {
            let source = fs::read_to_string(&path).unwrap();
            let production = source.split("#[cfg(test)]").next().unwrap_or(&source);
            assert!(
                !production.contains("SystemClock"),
                "{} constructs/imports a concrete Kernel clock in production",
                path.display()
            );
        }
    }
}
