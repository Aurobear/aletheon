use std::fs;
use std::path::{Path, PathBuf};

fn production_source(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path)
        .expect("production source")
        .split("#[cfg(test)]")
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn domain_ports_retain_only_authoritative_facades() {
    let source = production_source("src/core/domain_ports.rs");
    for contract in [
        "Arc<dyn AgoraService>",
        "Arc<dyn metacog::MetacogService>",
        "Arc<dyn corpus::CorpusService>",
        "Arc<dyn crate::service::harness_factory::CognitiveSessionFactory>",
    ] {
        assert!(
            source.contains(contract),
            "missing domain facade: {contract}"
        );
    }
    for concrete in [
        "MorphogenesisPipeline",
        "ToolRegistry",
        "HookRegistry",
        "ToolRunnerWithGuard",
        "LinearCognitiveSession",
    ] {
        assert!(
            !source.contains(concrete),
            "DomainPorts retained concrete implementation: {concrete}"
        );
    }
}

#[test]
fn request_turn_and_goal_paths_do_not_import_domain_implementations() {
    let files = [
        "src/impl/daemon/handler/mod.rs",
        "src/impl/daemon/handler/init.rs",
        "src/impl/daemon/handler/ports.rs",
        "src/impl/daemon/handler/tool_executor.rs",
        "src/impl/daemon/mcp_embedded.rs",
        "src/impl/runtime/provider_worker.rs",
        "src/service/request_use_cases.rs",
        "src/service/post_turn_projection.rs",
        "src/service/turn_pipeline.rs",
        "src/service/turn_runtime_ports.rs",
    ];
    let forbidden = [
        "mnemosyne::FactStore",
        "corpus::tools::tools::ToolRegistry",
        "corpus::HookRegistry",
        "ToolRunnerWithGuard",
        "metacog::r#impl",
        "MorphogenesisPipeline",
        "cognit::harness::linear",
        "LinearCognitiveSession",
    ];
    let mut violations = Vec::new();
    for file in files {
        let source = production_source(file);
        for needle in forbidden {
            if source.contains(needle) {
                violations.push(format!("{file}: {needle}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "domain facade bypasses:\n{}",
        violations.join("\n")
    );
}

#[test]
fn request_use_cases_retain_only_typed_runtime_and_domain_ports() {
    let source = production_source("src/service/request_use_cases.rs");
    for contract in [
        "Arc<dyn ExecutiveRuntimePort>",
        "Arc<dyn ReflectionMemoryPort>",
        "Arc<dyn ReflectionEnginePort>",
        "Arc<dyn SelfStatusPort>",
        "Arc<dyn SupplementalMemoryStatusPort>",
        "Arc<dyn RetentionAdminPort>",
        "Arc<dyn metacog::MetacogService>",
        "Arc<dyn corpus::CorpusService>",
    ] {
        assert!(
            source.contains(contract),
            "missing request port: {contract}"
        );
    }
    for concrete in [
        "AletheonExecutive",
        "EpisodicMemory",
        "SelfField",
        "CompositeMemoryHealth",
        "RetentionRepository",
        "RetentionCompactor",
        "cognit::core::reflector::Reflector",
    ] {
        assert!(
            !source.contains(concrete),
            "request use cases retained concrete domain state: {concrete}"
        );
    }
}

#[test]
fn exec_session_crosses_private_corpus_composition() {
    let source = production_source("src/service/exec_session.rs");
    assert!(
        source.contains("compose_exec_corpus"),
        "exec session does not use private Corpus composition"
    );
    for concrete in [
        "ToolRunnerWithGuard",
        "CorpusToolExecutor",
        "DefaultCorpusService",
        "HookRegistry",
        "default_tool_registry",
    ] {
        assert!(
            !source.contains(concrete),
            "exec session retained concrete Corpus ownership: {concrete}"
        );
    }
}

#[test]
fn turn_runtime_retain_only_typed_use_case_ports() {
    let source = production_source("src/service/turn_runtime_ports.rs");
    for contract in [
        "Arc<dyn SelfPolicyPort>",
        "Arc<dyn TurnConfigPort>",
        "Arc<dyn TurnHookPort>",
        "Arc<dyn StormStatePort>",
        "Arc<dyn ModelSelectionPort>",
        "Arc<dyn TurnApprovalPort>",
        "Arc<dyn GovernedTurnCapabilityPort>",
        "Arc<dyn TurnSessionStatePort>",
        "Arc<dyn TurnObservabilityPort>",
    ] {
        assert!(source.contains(contract), "missing turn port: {contract}");
    }
    for concrete in [
        "dasein::SelfField",
        "AletheonExecutive",
        "StormBreaker",
        "PendingApproval",
        "CapabilityResources",
        "SessionManager",
        "ModelRouter",
        "PerfCounter",
        "corpus::CorpusService",
        "mnemosyne::MemoryService",
    ] {
        assert!(
            !source.contains(concrete),
            "turn runtime retained concrete domain state: {concrete}"
        );
    }
}

#[test]
fn concrete_domain_construction_is_confined_to_composition_or_domain_tests() {
    let root = Path::new("src");
    let allowed = [
        PathBuf::from("src/impl/daemon/bootstrap"),
        PathBuf::from("src/impl/exec_corpus.rs"),
        PathBuf::from("src/service/harness_factory.rs"),
    ];
    let constructors = [
        "DefaultMetacogService::",
        "DefaultCorpusService::",
        "LinearCognitiveSession::new",
    ];
    let mut stack = vec![root.to_path_buf()];
    let mut violations = Vec::new();
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(path).expect("source directory") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let relative = path.to_path_buf();
            if allowed.iter().any(|prefix| relative.starts_with(prefix)) {
                continue;
            }
            let source = production_source(&relative);
            for constructor in constructors {
                if source.contains(constructor) {
                    violations.push(format!("{}: {constructor}", relative.display()));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "concrete domain construction escaped composition:\n{}",
        violations.join("\n")
    );
}
