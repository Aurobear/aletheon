//! Deterministic, bounded turn context assembly.

use crate::service::conscious_core_ports::LatestConsciousContextPort;
use crate::service::daemon_turn::helpers::{bounded_text_history, build_request_messages};
use async_trait::async_trait;
use fabric::{
    AgoraSpaceId, ConsciousContextProjection, ContextProjectionReceipt, Message, TurnRequest,
    WorkspaceContent,
};
use serde::Serialize;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

const MAX_FRAGMENT_CHARS: usize = 16 * 1024;
const MAX_INJECTED_CHARS: usize = 48 * 1024;
const MAX_SYSTEM_PREFIX_CHARS: usize = 128 * 1024;
const MAX_PROJECTED_ITEM_CHARS: usize = 4 * 1024;
const MAX_SELF_ITEM_CHARS: usize = 64;

#[derive(Clone, Debug, Default)]
pub struct ContextFragments {
    pub system_prefix: String,
    pub skills: String,
    pub conscious: Option<ConsciousContextProjection>,
}

#[derive(Clone, Debug)]
pub struct AssembledContext {
    pub messages: Vec<Message>,
    pub effective_user_message: String,
    pub projection_receipt: Option<ContextProjectionReceipt>,
}

#[derive(Debug, Error)]
pub enum ContextAssemblyError {
    #[error("context source failed: {0}")]
    Source(String),
}

#[async_trait]
pub trait ContextSource: Send + Sync {
    async fn load(&self, request: &TurnRequest) -> Result<ContextFragments, ContextAssemblyError>;
}

pub struct ContextAssembler {
    source: Arc<dyn ContextSource>,
}

pub struct ProductionContextSource {
    pub cached_prefix: Arc<Mutex<String>>,
    pub skill_loader: Arc<Mutex<corpus::SkillLoader>>,
    pub skill_router: Arc<Mutex<corpus::SkillRouter>>,
    pub conscious: Arc<dyn LatestConsciousContextPort>,
}

#[async_trait]
impl ContextSource for ProductionContextSource {
    async fn load(&self, request: &TurnRequest) -> Result<ContextFragments, ContextAssemblyError> {
        let system_prefix = format!(
            "{}\n\nCurrent working directory: {}\nTreat this as the user's current project. Do not scan unrelated host directories to guess a project.",
            self.cached_prefix.lock().await.clone(), request.working_dir.display()
        );
        let skills = {
            let loader = self.skill_loader.lock().await;
            let keywords = loader
                .plugins()
                .iter()
                .filter(|plugin| !plugin.keywords.is_empty())
                .map(|plugin| corpus::skill::keyword_matcher::SkillKeywords {
                    name: plugin.name.clone(),
                    keywords: plugin.keywords.clone(),
                    body: plugin.system_prompt.clone(),
                })
                .collect::<Vec<_>>();
            corpus::skill::keyword_matcher::match_skills(&request.input, &keywords).join("\n\n")
        };
        let suggestion = self
            .skill_router
            .lock()
            .await
            .suggest(&request.input, 0.6, 1)
            .first()
            .map(|item| {
                format!(
                    "Suggested /{} ({:.2}) — {}",
                    item.name, item.confidence, item.description
                )
            })
            .unwrap_or_default();
        let skills = [skills, suggestion]
            .into_iter()
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let conscious = self
            .conscious
            .latest_context(&AgoraSpaceId(request.session_id.clone()))
            .await
            .map_err(|error| ContextAssemblyError::Source(error.to_string()))?;
        conscious
            .validate()
            .map_err(|error| ContextAssemblyError::Source(error.to_string()))?;
        Ok(ContextFragments {
            system_prefix,
            skills,
            conscious: Some(conscious),
        })
    }
}

impl ContextAssembler {
    pub fn new(source: Arc<dyn ContextSource>) -> Self {
        Self { source }
    }

    pub async fn assemble(
        &self,
        request: &TurnRequest,
        canonical_history: &[Message],
    ) -> Result<AssembledContext, ContextAssemblyError> {
        let fragments = self.source.load(request).await?;
        let projection_receipt = fragments
            .conscious
            .as_ref()
            .map(|projection| projection.receipt.clone());
        let conscious = fragments
            .conscious
            .as_ref()
            .map(render_conscious_projection)
            .transpose()?;
        let mut effective = String::new();
        let mut remaining = MAX_INJECTED_CHARS;
        for (label, value) in [
            (
                "conscious-context",
                conscious.as_deref().unwrap_or_default(),
            ),
            ("skills", fragments.skills.as_str()),
        ] {
            append_fragment(&mut effective, label, value, &mut remaining);
        }
        if !effective.is_empty() {
            effective.push('\n');
        }
        effective.push_str(&request.input);
        let history = bounded_text_history(canonical_history);
        let messages = build_request_messages(
            truncate(&fragments.system_prefix, MAX_SYSTEM_PREFIX_CHARS),
            &history,
            effective.clone(),
        );
        Ok(AssembledContext {
            messages,
            effective_user_message: effective,
            projection_receipt,
        })
    }
}

#[derive(Serialize)]
struct ModelProjection<'a> {
    receipt: &'a ContextProjectionReceipt,
    self_view: ModelSelfView,
    selected: Vec<ModelSelectedContent>,
}

#[derive(Serialize)]
struct ModelSelfView {
    version: u64,
    mood: String,
    concerns: Vec<String>,
    projection: Option<String>,
    protentions: Vec<String>,
}

#[derive(Serialize)]
struct ModelSelectedContent {
    id: String,
    kind: &'static str,
    content: String,
}

fn render_conscious_projection(
    projection: &ConsciousContextProjection,
) -> Result<String, ContextAssemblyError> {
    projection
        .validate()
        .map_err(|error| ContextAssemblyError::Source(error.to_string()))?;
    let self_view = ModelSelfView {
        version: projection.self_view.version.0,
        mood: format!("{:?}", projection.self_view.mood),
        concerns: projection
            .self_view
            .concerns
            .iter()
            .map(|value| truncate(value, MAX_SELF_ITEM_CHARS))
            .collect(),
        projection: projection
            .self_view
            .projection
            .as_deref()
            .map(|value| truncate(value, MAX_SELF_ITEM_CHARS)),
        protentions: projection
            .self_view
            .protentions
            .iter()
            .map(|value| truncate(value, MAX_SELF_ITEM_CHARS))
            .collect(),
    };
    let selected = projection
        .latest_broadcast
        .iter()
        .flat_map(|broadcast| &broadcast.selected)
        .map(|candidate| ModelSelectedContent {
            id: candidate.id.0.to_string(),
            kind: content_kind(&candidate.content),
            content: truncate(
                &serde_json::to_string(&candidate.content).unwrap_or_default(),
                MAX_PROJECTED_ITEM_CHARS,
            ),
        })
        .collect();
    serde_json::to_string(&ModelProjection {
        receipt: &projection.receipt,
        self_view,
        selected,
    })
    .map_err(|error| ContextAssemblyError::Source(error.to_string()))
}

fn content_kind(content: &WorkspaceContent) -> &'static str {
    match content {
        WorkspaceContent::Observation(_) => "observation",
        WorkspaceContent::RecalledExperience(_) => "recalled_experience",
        WorkspaceContent::Evidence(_) => "evidence",
        WorkspaceContent::Hypothesis(_) => "hypothesis",
        WorkspaceContent::Prediction(_) => "prediction",
        WorkspaceContent::PredictionError(_) => "prediction_error",
        WorkspaceContent::Goal(_) => "goal",
        WorkspaceContent::Concern(_) => "concern",
        WorkspaceContent::CareConcern(_) => "care_concern",
        WorkspaceContent::Plan(_) => "plan",
        WorkspaceContent::ActionProposal(_) => "action_proposal",
        WorkspaceContent::ToolOutcome(_) => "tool_outcome",
        WorkspaceContent::GovernedActionOutcome(_) => "governed_action_outcome",
        WorkspaceContent::AgentResult(_) => "agent_result",
        WorkspaceContent::Reflection(_) => "reflection",
        WorkspaceContent::Extension { .. } => "extension",
    }
}

fn append_fragment(output: &mut String, label: &str, value: &str, remaining: &mut usize) {
    if value.trim().is_empty() || *remaining == 0 {
        return;
    }
    let bounded = truncate(value, MAX_FRAGMENT_CHARS.min(*remaining));
    *remaining = remaining.saturating_sub(bounded.chars().count());
    output.push_str(&format!("<{label}>\n{bounded}\n</{label}>\n"));
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}
