use std::fs;
use std::path::Path;

fn rust_files(path: &Path, output: &mut Vec<String>) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, output);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            output.push(fs::read_to_string(path).unwrap());
        }
    }
}

#[test]
fn production_has_one_authoritative_agent_control_implementation() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut sources = Vec::new();
    rust_files(&workspace.join("executive/src"), &mut sources);
    let implementations = sources
        .iter()
        .filter(|source| source.contains("impl AgentControlPort for"))
        .count();
    assert_eq!(implementations, 1);
    assert!(sources
        .iter()
        .any(|source| source.contains("impl AgentControlPort for AgentControlService")));

    let mut corpus = Vec::new();
    rust_files(&workspace.join("corpus/src"), &mut corpus);
    assert!(!corpus
        .iter()
        .any(|source| source.contains("SubAgentSpawner")));
}
