//! Private Corpus composition for non-daemon `exec` sessions.

use std::path::PathBuf;
use std::sync::Arc;

use corpus::security::approval::{ApprovalGate, TerminalApprovalGate};
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::executor::SandboxPreference;
use corpus::CorpusService;
use fabric::{Clock, PrincipalId};
use tokio::sync::Mutex;

pub(crate) struct ExecCorpusComposition {
    pub(crate) service: Arc<dyn CorpusService>,
    pub(crate) grant: corpus::ExtensionGrant,
}

pub(crate) async fn compose_exec_corpus(
    audit_path: PathBuf,
    sandbox: &str,
    clock: Arc<dyn Clock>,
    session_id: String,
) -> anyhow::Result<ExecCorpusComposition> {
    let registry = corpus::default_tool_registry();
    let approval: Arc<dyn ApprovalGate> = Arc::new(TerminalApprovalGate);
    let sandbox_preference = SandboxPreference::from_str(sandbox);
    tracing::info!(preference = ?sandbox_preference, "sandbox configured");
    let mut runner = ToolRunnerWithGuard::with_sandbox_preference(
        AuditLogger::new(audit_path)?,
        sandbox_preference,
        clock.clone(),
    )
    .with_approval_gate(approval);
    runner.on_new_turn(&uuid::Uuid::new_v4().to_string());

    let raw_executor = Arc::new(corpus::CorpusToolExecutor::new(
        registry.clone(),
        Arc::new(Mutex::new(runner)),
        clock.clone(),
    ));
    let service: Arc<dyn CorpusService> = Arc::new(corpus::DefaultCorpusService::from_runtime(
        registry.clone(),
        raw_executor,
        Arc::new(Mutex::new(corpus::HookRegistry::new(clock))),
    ));
    let descriptors = corpus::discover_tool_extensions(&registry).await?;
    let grant = corpus::ExtensionGrant {
        grant_id: uuid::Uuid::new_v4().to_string(),
        principal: PrincipalId("exec".into()),
        session_id,
        agent_id: None,
        capabilities: descriptors
            .iter()
            .flat_map(|descriptor| descriptor.capabilities.clone())
            .collect(),
        resources: Default::default(),
    };
    Ok(ExecCorpusComposition { service, grant })
}
