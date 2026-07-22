//! Initial session parameter registration for daemon bootstrap.

use std::sync::Arc;

use fabric::Clock;
use fabric::LlmProvider;
use serde_json::json;
use tracing::info;

use crate::core::session_gateway::ParamRegistry;

pub(super) async fn register_initial_params(
    param_registry: &ParamRegistry,
    clock: Arc<dyn Clock>,
    data_dir: std::path::PathBuf,
    model: String,
    llm: Arc<dyn LlmProvider>,
    sandbox_pref: String,
) {
    let started_at = clock.mono_now();
    param_registry
        .declare(
            "session.uptime_secs",
            "session",
            "Daemon uptime in seconds",
            move || {
                let elapsed_ms = clock.mono_now().0.saturating_sub(started_at.0);
                json!(elapsed_ms / 1000)
            },
        )
        .await;
    param_registry
        .declare(
            "session.data_dir",
            "session",
            "Data directory path",
            move || json!(data_dir.to_string_lossy()),
        )
        .await;
    let model_for_param = model;
    param_registry
        .declare("llm.model", "llm", "Current LLM model in use", move || {
            json!(model_for_param)
        })
        .await;
    let provider_name = llm.name().to_string();
    param_registry
        .declare(
            "llm.provider",
            "llm",
            "Current LLM provider name",
            move || json!(provider_name),
        )
        .await;
    let sandbox_pref_for_param = sandbox_pref;
    param_registry
        .declare(
            "sandbox.preference",
            "sandbox",
            "Current sandbox mode",
            move || json!(sandbox_pref_for_param),
        )
        .await;
    param_registry
        .declare("session.rss_kb", "session", "Resident memory in KB", || {
            let status = std::fs::read_to_string("/proc/self/status").ok();
            let rss = status.and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmRSS:"))
                    .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            });
            json!(rss.unwrap_or(0))
        })
        .await;
    info!("Registered {} initial params", 6);
}
