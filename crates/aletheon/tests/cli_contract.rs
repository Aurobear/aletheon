use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{self, Value};
use std::process::Command;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_sources(root: &Path) -> Vec<(PathBuf, String)> {
    fn collect(directory: &Path, output: &mut Vec<(PathBuf, String)>) {
        for entry in fs::read_dir(directory).expect("source directory is readable") {
            let entry = entry.expect("source entry is readable");
            let path = entry.path();
            if path.is_dir() {
                collect(&path, output);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                let source = fs::read_to_string(&path).expect("Rust source is UTF-8");
                output.push((path, source));
            }
        }
    }

    let mut output = Vec::new();
    collect(root, &mut output);
    output
}

#[test]
fn a_entry_002_compatibility_parser_is_deleted() {
    let root = repository_root();
    let interact = root.join("crates/interact");
    assert!(
        !interact.join("src/tui/cli.rs").exists(),
        "the compatibility parser source must be deleted"
    );

    let offenders = rust_sources(&interact.join("src"))
        .into_iter()
        .filter(|(_, source)| source.contains("derive(Parser)") || source.contains("::parse()"))
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "Interact must not own a production argument parser: {offenders:?}"
    );
}

#[test]
fn a_entry_003_assembly_has_no_interact_cli_imports() {
    let root = repository_root();
    let offenders = rust_sources(&root.join("crates/aletheon/src"))
        .into_iter()
        .filter(|(_, source)| {
            source.contains("interact::cli") || source.contains("interact::tui::cli")
        })
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "assembly must not depend on the removed presentation parser: {offenders:?}"
    );
}

#[test]
fn u_cli_004_interact_cli_has_zero_production_callers() {
    let root = repository_root();
    let mut sources = rust_sources(&root.join("crates/interact/src"));
    sources.extend(rust_sources(&root.join("crates/aletheon/src")));
    let forbidden = [
        "crate::cli",
        "interact::cli",
        "interact::tui::cli",
        "pub use tui::cli",
        "pub mod cli;",
    ];
    let offenders = sources
        .into_iter()
        .filter_map(|(path, source)| {
            forbidden
                .iter()
                .any(|pattern| source.contains(pattern))
                .then_some(path)
        })
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "removed compatibility module still has production references: {offenders:?}"
    );
    assert!(
        root.join("crates/interact/src/single_message.rs").exists(),
        "the one-shot transport must remain as a parser-free adapter"
    );
}
#[test]
fn plain_version_test() {
    let output = Command::new(env!("CARGO_BIN_EXE_aletheon"))
        .arg("version")
        .output()
        .expect("aletheon binary should run");
    assert!(output.status.success(), "status must be success");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let expected = format!("aletheon {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(stdout, expected, "stdout should be plain version string");
    assert!(output.stderr.is_empty(), "stderr should be empty");
}

#[test]
fn json_version_test() {
    let output = Command::new(env!("CARGO_BIN_EXE_aletheon"))
        .args(["version", "--json"])
        .output()
        .expect("aletheon binary should run");
    assert!(output.status.success(), "status must be success");
    assert!(output.stderr.is_empty(), "stderr must be empty");
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout is valid JSON");
    let obj = value.as_object().expect("must be a JSON object");
    // The version JSON carries deployment provenance (source_revision from
    // GIT_COMMIT_SHA, config_hash from CONFIG_HASH) alongside the protocol
    // contract fields.  Core keys must remain; the provenance keys are part of
    // the accepted install-state contract (M10 spec §11.2).
    let expected_keys = [
        "config_hash",
        "name",
        "protocol_version",
        "schema_version",
        "source_revision",
        "version",
    ];
    let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(keys, expected_keys, "exact key set must match sorted");
    assert_eq!(
        obj["schema_version"]
            .as_u64()
            .expect("schema_version must be u64"),
        1u64
    );
    assert_eq!(
        obj["name"].as_str().expect("name must be string"),
        "aletheon"
    );
    assert_eq!(
        obj["version"].as_str().expect("version must be string"),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        obj["protocol_version"]
            .as_u64()
            .expect("protocol_version must be u64"),
        ::contracts::CLIENT_PROTOCOL_VERSION as u64
    );
}
