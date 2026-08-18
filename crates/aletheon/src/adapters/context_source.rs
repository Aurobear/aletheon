//! Production adapters for Application turn-context sources.

use application::turn::context::{
    format_recall_context, ContextAssemblyError, ContextFragments, ContextMemoryRecallPort,
    ContextMemoryRecallRequest, ContextSource, SkillContextPort,
};
use async_trait::async_trait;
use contracts::{AgoraSpaceId, LatestConsciousContextPort, TurnRequest};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

pub struct CorpusSkillContext {
    loader: Arc<Mutex<corpus::SkillLoader>>,
    router: Arc<Mutex<corpus::SkillRouter>>,
}

impl CorpusSkillContext {
    pub fn new(
        loader: Arc<Mutex<corpus::SkillLoader>>,
        router: Arc<Mutex<corpus::SkillRouter>>,
    ) -> Self {
        Self { loader, router }
    }
}

#[async_trait]
impl SkillContextPort for CorpusSkillContext {
    async fn context(&self, input: &str) -> Result<String, ContextAssemblyError> {
        let loader = self.loader.lock().await;
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
        let matched = corpus::skill::keyword_matcher::match_skills(input, &keywords).join("\n\n");
        drop(loader);
        let suggestion = self
            .router
            .lock()
            .await
            .suggest(input, 0.6, 1)
            .first()
            .map(|item| {
                format!(
                    "Suggested /{} ({:.2}) — {}",
                    item.name, item.confidence, item.description
                )
            })
            .unwrap_or_default();
        Ok([matched, suggestion]
            .into_iter()
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

pub struct ProductionContextSource {
    pub cached_prefix: Arc<Mutex<String>>,
    pub skills: Arc<dyn SkillContextPort>,
    pub conscious: Arc<dyn LatestConsciousContextPort>,
    pub memory_service: Option<Arc<dyn ContextMemoryRecallPort>>,
    pub recall_enabled: bool,
    pub recall_max_items: usize,
    pub recall_max_bytes: usize,
    pub recall_timeout_ms: u64,
}

pub fn working_directory_policy_prompt(working_dir: &std::path::Path) -> String {
    format!(
        "Current working directory: {}\nTreat this as the user's current project. Do not scan unrelated host directories to guess a project. Mutation tools are confined to this directory by the configured sandbox/working-directory policy. Read-only errors for paths outside it do not establish host mount state because host mount state was not checked; do not change host mounts. Relaunch from the intended working directory or choose a path inside this directory.",
        working_dir.display()
    )
}

#[async_trait]
impl ContextSource for ProductionContextSource {
    async fn load(&self, request: &TurnRequest) -> Result<ContextFragments, ContextAssemblyError> {
        let prefix_and_skills = async {
            let system_prefix = format!(
                "{}\n\n{}",
                self.cached_prefix.lock().await.clone(),
                working_directory_policy_prompt(request.context.workspace.cwd())
            );
            let skills = self.skills.context(&request.input).await?;
            Ok::<_, ContextAssemblyError>((system_prefix, skills))
        };
        let conscious = async {
            match self
                .conscious
                .latest_context(&AgoraSpaceId(request.context.thread_id.0.clone()))
                .await
            {
                Ok(projection) => {
                    projection
                        .validate()
                        .map_err(|error| ContextAssemblyError::Source(error.to_string()))?;
                    Ok::<_, ContextAssemblyError>(Some(projection))
                }
                Err(_) => Ok(None),
            }
        };
        let memory_context = async {
            if self.recall_enabled {
                if let Some(ref service) = self.memory_service {
                    let recall = service.recall(ContextMemoryRecallRequest {
                        principal_id: request.context.principal_id.clone(),
                        session_id: request.context.thread_id.0.clone(),
                        working_dir: request.context.workspace.cwd().to_owned(),
                        query: request.input.clone(),
                        max_items: self.recall_max_items,
                        max_content_bytes: self.recall_max_bytes,
                    });
                    return match tokio::time::timeout(
                        Duration::from_millis(self.recall_timeout_ms),
                        recall,
                    )
                    .await
                    {
                        Ok(Ok(set)) if !set.items.is_empty() => format_recall_context(&set),
                        Ok(Ok(_)) | Ok(Err(_)) | Err(_) => String::new(),
                    };
                }
            }
            String::new()
        };
        let (prefix_and_skills, conscious, memory_context) =
            tokio::join!(prefix_and_skills, conscious, memory_context);
        let (system_prefix, skills) = prefix_and_skills?;
        Ok(ContextFragments {
            system_prefix,
            skills,
            conscious: conscious?,
            memory_context,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn observe_and_resume_turn(
    memory: &mnemosyne::MemoryGatewayService,
    conscious: Option<&Arc<dyn crate::composition::conscious_workspace::ConsciousTurnPort>>,
    sessions: &runtime::session_service::SessionService,
    principal: &contracts::PrincipalId,
    memory_session_id: &str,
    conscious_session_id: &str,
    canonical_session_id: &str,
    turn_id: &str,
    working_dir: &std::path::Path,
    process_id: contracts::ProcessId,
    operation_id: contracts::OperationId,
    message: &str,
) -> anyhow::Result<Vec<contracts::Message>> {
    let memory_observation = adapters_gbrain::context_memory::observe_native_user(
        memory,
        principal,
        memory_session_id,
        turn_id,
        working_dir,
        message,
    );
    let conscious_observation = crate::adapters::conscious::turn_workspace::observe_turn_input(
        conscious,
        conscious_session_id,
        process_id,
        operation_id,
        message,
    );
    let canonical_session = contracts::SessionId(canonical_session_id.to_owned());
    let history = adapters_sqlite::session::turn_history::resume_before_current_user(
        sessions,
        &canonical_session,
        message,
    );
    let (memory_observation, conscious_observation, history) =
        tokio::join!(memory_observation, conscious_observation, history);
    if let Err(error) = memory_observation {
        tracing::warn!(%error, "native user memory observation degraded");
    }
    conscious_observation?;
    history
}
