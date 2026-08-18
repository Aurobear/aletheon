use async_trait::async_trait;
use corpus::hook::{HookContext, HookResult};
use std::collections::HashMap;

#[async_trait]
pub trait TurnHookPort: Send + Sync {
    async fn execute(&self, context: HookContext) -> HookResult;
}

pub async fn prepare_turn_input(
    hooks: &dyn TurnHookPort,
    message: &str,
    session_id: &str,
    turn_count: usize,
    workspace_root: &std::path::Path,
    repo_hooks_trusted: bool,
    mut prefix: String,
) -> Result<String, application::turn::outcome::TurnPipelineRejection> {
    let authority_metadata = HashMap::from([
        (
            "workspace_root".into(),
            workspace_root.display().to_string(),
        ),
        ("repo_hooks_trusted".into(), repo_hooks_trusted.to_string()),
    ]);
    hooks
        .execute(HookContext {
            point: corpus::hook::HookPoint::UserPromptSubmit,
            session_id: session_id.to_string(),
            turn_count,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: Some(message.to_string()),
            metadata: authority_metadata.clone(),
        })
        .await;
    let result = hooks
        .execute(HookContext {
            point: corpus::hook::HookPoint::PreTurn,
            session_id: session_id.to_string(),
            turn_count,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: Some(message.to_string()),
            metadata: authority_metadata,
        })
        .await;
    match result {
        HookResult::Block { reason } => {
            tracing::warn!(reason = %reason, "PreTurn hook blocked");
            Err(application::turn::outcome::TurnPipelineRejection::HookBlocked { reason })
        }
        HookResult::Inject(text) => {
            prefix.push_str(&text);
            prefix.push('\n');
            prefix.push_str(message);
            Ok(prefix)
        }
        _ => {
            prefix.push_str(message);
            Ok(prefix)
        }
    }
}
