use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("fabric manifest must live under crates/fabric")
        .to_path_buf()
}

fn data_rows(path: &Path) -> Vec<Vec<String>> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

#[test]
fn a_dep_001_existing_architecture_gates_and_migration_actions_are_preserved() {
    let root = repository_root();
    for relative in [
        "config/architecture-allowlist.txt",
        "config/architecture-dependencies.txt",
        "config/architecture-path-inventory.txt",
        "config/architecture/contract-migrations.tsv",
        "config/architecture/acceptance-ids.tsv",
        "scripts/libexec/aletheon/architecture-check.sh",
        "tests/suites/architecture/architecture_check.sh",
    ] {
        assert!(root.join(relative).is_file(), "missing {relative}");
    }

    let matrix = data_rows(&root.join("config/architecture/contract-migrations.tsv"));
    let actions = matrix
        .iter()
        .map(|row| row.get(3).expect("migration action").as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actions,
        BTreeSet::from(["delete", "existing", "extend", "new", "project", "v2"])
    );
}

#[test]
fn a_dep_002_forbidden_edges_parsers_and_writers_are_zero_or_ratcheted() {
    let root = repository_root();
    let metrics = fs::read_to_string(root.join("config/architecture/metrics.env"))
        .expect("architecture metrics");
    assert!(metrics.contains("FORBIDDEN_DEPENDENCY_EDGES=0"));
    assert!(metrics.contains("PRODUCTION_CLI_PARSERS=1"));
    assert!(metrics.contains("SESSION_APPEND_WRITERS=2"));

    let checker = fs::read_to_string(root.join("scripts/libexec/aletheon/architecture-check.sh"))
        .expect("architecture checker");
    for contract in [
        "interact\":\n        metrics[\"FORBIDDEN_DEPENDENCY_EDGES\"]",
        "cognit\":\n        metrics[\"FORBIDDEN_DEPENDENCY_EDGES\"]",
        "hardware::grpc::",
        "PRODUCTION_CLI_PARSERS",
        "SESSION_APPEND_WRITERS",
        "duplicate RPC method arms",
    ] {
        assert!(checker.contains(contract), "checker lacks {contract}");
    }

    let fixtures = fs::read_to_string(root.join("tests/suites/architecture/architecture_check.sh"))
        .expect("architecture negative fixtures");
    for fixture in [
        "additional production CLI parser",
        "additional Session append writer",
        "hardware adapter bypass",
        "unregistered wire surface",
        "unregistered persistence migration",
    ] {
        assert!(
            fixtures.contains(fixture),
            "negative fixture lacks {fixture}"
        );
    }
}

#[test]
fn a_dep_003_fabric_public_types_require_governed_metadata_after_baseline() {
    let root = repository_root();
    let snapshot_path = root.join("config/architecture/fabric-public-types.tsv");
    let source = fs::read_to_string(&snapshot_path).expect("Fabric public type snapshot");
    let baseline_count = source
        .lines()
        .find_map(|line| line.strip_prefix("# baseline_count="))
        .expect("baseline_count")
        .parse::<usize>()
        .expect("numeric baseline_count");
    let rows = data_rows(&snapshot_path);
    let baseline_rows = rows
        .iter()
        .filter(|row| row.get(3).map(String::as_str) == Some("baseline"))
        .count();
    assert_eq!(baseline_rows, baseline_count);

    let mut keys = BTreeSet::new();
    for row in rows {
        assert_eq!(row.len(), 7, "invalid Fabric public type row: {row:?}");
        assert!(
            root.join(&row[0]).is_file(),
            "missing source path {}",
            row[0]
        );
        assert!(keys.insert((row[0].clone(), row[1].clone(), row[2].clone())));
        match row[3].as_str() {
            "baseline" => assert_eq!(
                (&row[4], &row[5], &row[6]),
                (&"-".into(), &"-".into(), &"-".into())
            ),
            "governed" => {
                assert!(row[4..=6]
                    .iter()
                    .all(|value| !value.is_empty() && value != "-"));
                let decision = row[6].split('#').next().expect("decision path");
                assert!(root.join(decision).is_file(), "missing decision {decision}");
            }
            other => panic!("invalid provenance {other}"),
        }
    }
}
