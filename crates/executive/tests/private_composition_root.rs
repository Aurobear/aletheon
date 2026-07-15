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

#[test]
fn composition_is_private_and_bootstrap_confined() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bootstrap = root.join("src/impl/daemon/bootstrap");
    let expected = [
        "mod.rs",
        "storage.rs",
        "google.rs",
        "runtime.rs",
        "channels.rs",
        "request.rs",
    ];
    for file in expected {
        assert!(
            bootstrap.join(file).is_file(),
            "missing bootstrap stage: {file}"
        );
    }

    let mut locations = Vec::new();
    for path in rust_files(&root.join("src")) {
        let source = fs::read_to_string(&path).expect("source file");
        if source.contains("DaemonComposition") {
            locations.push(path);
        }
    }
    assert!(!locations.is_empty(), "private composition root is missing");
    assert!(
        locations.iter().all(|path| path.starts_with(&bootstrap)),
        "DaemonComposition escaped bootstrap: {locations:?}"
    );

    let init = fs::read_to_string(root.join("src/impl/daemon/handler/init.rs")).unwrap();
    assert!(
        init.lines().count() <= 250,
        "handler init is no longer thin"
    );
    let module = fs::read_to_string(bootstrap.join("mod.rs")).unwrap();
    assert!(
        module.lines().count() <= 700,
        "bootstrap root module is too large"
    );
    assert!(
        !module.contains("derive(Clone)"),
        "composition root must not be Clone"
    );
}
