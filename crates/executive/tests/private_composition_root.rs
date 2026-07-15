use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).expect("read production source") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn retired_god_container_cannot_return() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(!root.join("src/core/core_systems.rs").exists());
    let mut violations = Vec::new();
    for path in rust_files(&root.join("src")) {
        let source = fs::read_to_string(&path).expect("source file");
        for forbidden in ["CoreSystems", ".subsystems"] {
            if source.contains(forbidden) {
                violations.push(format!("{}: {forbidden}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "retired composition container escaped:\n{}",
        violations.join("\n")
    );
}

#[test]
fn concrete_groups_are_crate_private() {
    let core = fs::read_to_string("src/core/mod.rs").expect("core module");
    for module in [
        "corpus_group",
        "memory_group",
        "security_group",
        "session_group",
    ] {
        assert!(core.contains(&format!("pub(crate) mod {module};")));
    }
}
