use std::fs;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("metacog manifest must live under crates/metacog")
        .to_path_buf()
}

#[test]
fn a_delete_001_deprecation_ledger_has_only_explicit_pending_rows() {
    let root = repository_root();
    let ledger = fs::read_to_string(root.join("config/architecture/compatibility-debt.tsv"))
        .expect("compatibility ledger");
    let mut rows = 0;
    for line in ledger
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 7, "invalid compatibility row: {line}");
        assert!(!fields[3].is_empty(), "missing reason: {line}");
        assert!(
            !fields[4].is_empty(),
            "missing canonical replacement: {line}"
        );
        assert_eq!(fields[6], "no-v1-rows-and-supported-upgrade-window-closed");
        rows += 1;
    }
    assert_eq!(rows, 2, "unexpected compatibility debt was added");

    let source =
        fs::read_to_string(root.join("crates/metacog/src/lib.rs")).expect("metacog facade");
    assert!(!source.contains("pub mod hil_evidence_verifier"));
    assert!(!source.contains("pub mod outcome_verifier"));
}

#[test]
fn a_delete_002_readme_stable_capabilities_have_code_tests_and_recovery_evidence() {
    let root = repository_root();
    let readme = fs::read_to_string(root.join("README.md")).expect("README");
    let matrix = readme
        .split("### 6.2 Stable")
        .next()
        .expect("capability matrix");
    let mut stable_rows = 0;
    for line in matrix.lines().filter(|line| line.contains("✅ Stable")) {
        let columns: Vec<_> = line.split('|').map(str::trim).collect();
        assert_eq!(columns.len(), 7, "malformed stable capability row: {line}");
        stable_rows += 1;
        for (column, label) in [(3, "production code"), (4, "E2E"), (5, "recovery")] {
            let anchor = columns[column].trim_matches('`');
            assert!(
                !anchor.is_empty() && anchor != "—",
                "missing {label} anchor: {line}"
            );
            assert!(
                root.join(anchor).is_file(),
                "{label} anchor is not a file: {anchor}"
            );
        }
        let e2e = fs::read_to_string(root.join(columns[4].trim_matches('`')))
            .expect("stable E2E evidence");
        assert!(
            e2e.contains("#[test]") || e2e.contains("#[tokio::test]"),
            "E2E anchor contains no executable test: {line}"
        );
        let recovery = fs::read_to_string(root.join(columns[5].trim_matches('`')))
            .expect("stable recovery evidence")
            .to_ascii_lowercase();
        assert!(
            [
                "restart",
                "recover",
                "replay",
                "reconnect",
                "idempot",
                "stale",
                "cooldown",
                "fail_closed",
                "private_scratch",
            ]
            .iter()
            .any(|term| recovery.contains(term)),
            "recovery anchor lacks restart/replay/equivalent evidence: {line}"
        );
    }
    assert_eq!(
        stable_rows, 13,
        "Stable capability inventory changed without evidence review"
    );
    let stable = readme
        .split("### 6.2 Stable")
        .nth(1)
        .expect("stable section");
    assert!(stable.contains("EventSourcedSessionStore"));
    assert!(!stable.contains("HashMap-based session registry"));
    assert!(!stable.contains("adapters/session/store.rs"));
}
