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
fn binary_composition_uses_owner_domain_services() {
    let source = production_source("../aletheon/src/wiring/domain.rs");
    for contract in [
        "agora: Arc<dyn AgoraService>",
        "metacog: Arc<dyn metacog::MetacogService>",
        "corpus: Arc<dyn corpus::CorpusService>",
    ] {
        assert!(
            source.contains(contract),
            "missing owner domain service: {contract}"
        );
    }
    assert!(
        !source.contains("struct DomainPorts"),
        "legacy Executive DomainPorts bag returned"
    );
}

#[test]
fn request_turn_and_goal_paths_do_not_import_domain_implementations() {
    let files = [
        "../aletheon/src/wiring/daemon/handler/mod.rs",
        "../aletheon/src/wiring/daemon/handler/init.rs",
        "../aletheon/src/wiring/daemon/handler/ports.rs",
        "../aletheon/src/wiring/daemon/handler/tool_executor.rs",
        "../aletheon/src/wiring/daemon/mcp_embedded.rs",
        "src/wiring/adapters/runtime/provider_worker.rs",
        "src/wiring/application/request_use_cases.rs",
        "src/wiring/application/admin_service.rs",
        "src/wiring/application/post_turn_projection.rs",
        "src/wiring/application/turn_pipeline.rs",
        "src/wiring/application/turn_runtime_ports.rs",
    ];
    let forbidden = [
        "mnemosyne::runtime::FactStore",
        "corpus::tools::tools::ToolRegistry",
        "corpus::HookRegistry",
        "ToolRunnerWithGuard",
        "metacog::r#impl",
        "MorphogenesisPipeline",
        "cognit::harness::linear",
        "LinearCognitiveSession",
        "AletheonCognitiveRuntime",
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
fn admin_and_post_turn_retain_runtime_facades_not_executive() {
    for (file, contract) in [
        (
            "src/wiring/application/admin_service.rs",
            "Arc<dyn AdminRuntimePort>",
        ),
        (
            "src/wiring/application/post_turn_projection.rs",
            "Arc<dyn PostTurnRuntimePort>",
        ),
    ] {
        let source = production_source(file);
        assert!(
            source.contains(contract),
            "missing runtime facade in {file}"
        );
        assert!(
            !source.contains("AletheonCognitiveRuntime"),
            "{file} retained concrete AletheonCognitiveRuntime"
        );
    }
}

#[test]
fn request_use_cases_retain_only_typed_runtime_and_domain_ports() {
    let source = production_source("src/wiring/application/request_use_cases.rs");
    for contract in [
        "Arc<dyn CognitiveRuntimePort>",
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
        "AletheonCognitiveRuntime",
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
    let source = production_source("../aletheon/src/wiring/exec_session.rs");
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
    let source = production_source("src/wiring/application/turn_runtime_ports.rs");
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
        "AletheonCognitiveRuntime",
        "StormBreaker",
        "PendingApproval",
        "CapabilityResources",
        "ContextWorkingSet",
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
        PathBuf::from("src/wiring/daemon/bootstrap"),
        PathBuf::from("src/wiring/composition/exec_corpus.rs"),
        PathBuf::from("src/wiring/application/harness_factory.rs"),
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
