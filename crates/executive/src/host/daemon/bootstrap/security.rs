//! Tool security composition for the daemon request runtime.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::executor::{create_executor_with_front_backend, SandboxPreference};

pub(super) fn build_tool_runner(
    data_dir: &Path,
    working_dir: &str,
    sandbox_preference: &str,
    grok_hardening: &crate::composition::config::GrokHardeningConfig,
    sandbox_profiles: fabric::SandboxProfiles,
    approval_gate: Arc<dyn corpus::security::approval::ApprovalGate>,
    event_bus: Option<&Arc<fabric::CanonicalEventBus>>,
    clock: Arc<dyn fabric::Clock>,
) -> Result<Arc<tokio::sync::Mutex<ToolRunnerWithGuard>>> {
    let sandbox_pref = SandboxPreference::from_str(sandbox_preference);
    let mut structured: Option<Arc<dyn corpus::security::StructuredToolSandbox>> = None;
    let front_backend: Option<Box<dyn fabric::SandboxBackend>> = if grok_hardening.execd {
        let binary_path = std::env::var_os("ALETHEON_EXECD_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|path| path.parent().map(|parent| parent.join("execd")))
                    .unwrap_or_else(|| std::path::PathBuf::from("execd"))
            });
        let workspace = std::path::PathBuf::from(working_dir)
            .canonicalize()
            .context("canonicalize execd workspace root")?;
        let backend = crate::adapters::channel::execd_client::ExecdSandboxBackend::new(
            crate::adapters::channel::execd_client::ExecdConfig {
                binary_path: binary_path.to_string_lossy().into_owned(),
                shared_secret: format!(
                    "{}{}",
                    uuid::Uuid::new_v4().simple(),
                    uuid::Uuid::new_v4().simple()
                ),
                startup_timeout: std::time::Duration::from_secs(5),
                request_timeout: std::time::Duration::from_secs(30),
                workspace_roots: vec![workspace],
            },
        );
        structured = Some(Arc::new(backend.clone()));
        Some(Box::new(backend))
    } else {
        None
    };
    let sandbox = create_executor_with_front_backend(sandbox_pref, clock.clone(), front_backend);
    let mut runner = ToolRunnerWithGuard::new(
        sandbox,
        AuditLogger::new(data_dir.join("audit.jsonl"))?,
        clock,
    )
    .with_approval_gate(approval_gate);
    if let Some(structured) = structured {
        runner = runner.with_structured_sandbox(structured);
    }
    if grok_hardening.sandbox_profiles {
        runner = runner.with_sandbox_profiles(sandbox_profiles);
    }
    if let Some(bus) = event_bus {
        runner = runner.with_event_bus(bus.clone());
    }
    Ok(Arc::new(tokio::sync::Mutex::new(runner)))
}
