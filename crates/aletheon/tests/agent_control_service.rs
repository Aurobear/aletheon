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
    let mut application = Vec::new();
    rust_files(&workspace.join("application/src"), &mut application);
    let implementations = application
        .iter()
        .filter(|source| source.contains("impl AgentControlPort for"))
        .count();
    assert_eq!(implementations, 1);
    assert!(application
        .iter()
        .any(|source| source.contains("impl AgentControlPort for AgentService")));

    let mut composition = Vec::new();
    rust_files(
        &workspace.join("aletheon/src/composition/agent_control"),
        &mut composition,
    );
    assert_eq!(
        composition
            .iter()
            .filter(|source| source.contains("impl AgentControlPort for"))
            .count(),
        1
    );
    assert!(composition
        .iter()
        .any(|source| source.contains("impl AgentControlPort for RuntimeAgentControlFacade")));
    assert!(!composition
        .iter()
        .any(|source| source.contains("impl AgentControlPort for AgentHostAdapter")));

    let mut corpus = Vec::new();
    rust_files(&workspace.join("corpus/src"), &mut corpus);
    assert!(!corpus
        .iter()
        .any(|source| source.contains("SubAgentSpawner")));
}
