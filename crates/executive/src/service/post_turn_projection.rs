//! Non-blocking projections of an already-settled turn.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use cognit::core::reflector::Reflector;
use fabric::hook::{HookContext, HookPoint};
use fabric::{AgoraOps, Clock, ReflectionTrigger};
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};
use mnemosyne::{AutoMemory, MemoryService, RecallMemory};
use tracing::info;

use crate::core::core_systems::CoreSystems;

#[derive(Clone, Debug)]
pub struct PostTurnOutcome {
    pub session_id: String,
    pub input: String,
    pub output: String,
    pub turn: usize,
    pub succeeded: bool,
    pub tool_calls_made: usize,
    pub tool_errors: usize,
    pub elapsed_ms: u64,
    pub iterations: usize,
    pub completed_normally: bool,
    pub agora_start_version: u64,
}

#[async_trait]
pub trait PostTurnProjection: Send + Sync {
    async fn project(&self, outcome: PostTurnOutcome) -> anyhow::Result<()>;
}

pub struct PostTurnDispatch {
    pub projector: Arc<dyn PostTurnProjection>,
    pub outcome: PostTurnOutcome,
}

pub struct ProductionPostTurnProjection {
    run_hook: Arc<HookProjectionFn>,
    memory_service: Arc<dyn MemoryService>,
    auto_memory: Arc<tokio::sync::Mutex<AutoMemory>>,
    reflector: Reflector,
    episodic_memory: Arc<tokio::sync::Mutex<mnemosyne::episodic::EpisodicMemory>>,
    clock: Arc<dyn Clock>,
    runtime: Arc<tokio::sync::Mutex<crate::core::orchestrator::AletheonExecutive>>,
    evolution_pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,
    agora: Arc<dyn AgoraOps>,
    recall_memory: Arc<tokio::sync::Mutex<RecallMemory>>,
}

type HookProjectionFn =
    dyn Fn(HookContext) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;

impl ProductionPostTurnProjection {
    pub fn new(subsystems: Arc<CoreSystems>) -> Self {
        let memory = &subsystems.memory;
        let hook_registry = subsystems.corpus.hook_registry.clone();
        let run_hook: Arc<HookProjectionFn> = Arc::new(move |context| {
            let hook_registry = hook_registry.clone();
            Box::pin(async move {
                hook_registry.lock().await.execute(&context).await;
            })
        });
        Self {
            run_hook,
            memory_service: memory.memory_service.clone(),
            auto_memory: memory.auto_memory.clone(),
            reflector: subsystems.reflector.clone(),
            episodic_memory: memory.episodic_memory.clone(),
            clock: subsystems.kernel.clock(),
            runtime: Arc::clone(&subsystems.runtime),
            evolution_pipeline: subsystems.pipeline.clone(),
            agora: subsystems.domains.agora(),
            recall_memory: memory.recall_memory.clone(),
        }
    }
}

#[async_trait]
impl PostTurnProjection for ProductionPostTurnProjection {
    async fn project(&self, outcome: PostTurnOutcome) -> anyhow::Result<()> {
        let mut failures = Vec::new();
        self.run_post_turn_hook(&outcome).await;

        if outcome.succeeded {
            if let Err(error) = self.record_assistant_message(&outcome).await {
                failures.push(format!("assistant memory: {error}"));
            }
            if let Err(error) = self.extract_auto_memory(&outcome).await {
                failures.push(format!("auto memory: {error}"));
            }
        }
        if let Err(error) = self.record_reflection(&outcome).await {
            failures.push(format!("reflection: {error}"));
        }
        if let Err(error) = self.run_evolution(&outcome).await {
            failures.push(format!("evolution: {error}"));
        }
        if let Err(error) = self.persist_agora_commits(&outcome).await {
            failures.push(format!("agora: {error}"));
        }

        if failures.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(failures.join("; "))
        }
    }
}

impl ProductionPostTurnProjection {
    async fn run_post_turn_hook(&self, outcome: &PostTurnOutcome) {
        let context = HookContext {
            point: HookPoint::PostTurn,
            session_id: outcome.session_id.clone(),
            turn_count: outcome.turn,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };
        (self.run_hook)(context).await;
    }

    async fn record_assistant_message(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()> {
        let observed_at = fabric::wall_to_datetime(self.clock.wall_now());
        self.memory_service
            .record(mnemosyne::ExperienceEvent::Message {
                session: outcome.session_id.clone(),
                role: "assistant".into(),
                content: outcome.output.clone(),
                metadata: mnemosyne::MemoryMetadata::local(
                    format!("message:{}:assistant:{}", outcome.session_id, outcome.turn),
                    format!("{}:assistant:{}", outcome.session_id, outcome.turn),
                    observed_at,
                ),
            })
            .await?;
        let context = HookContext {
            point: HookPoint::OnMemoryStore,
            session_id: outcome.session_id.clone(),
            turn_count: outcome.turn,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };
        (self.run_hook)(context).await;
        Ok(())
    }

    async fn extract_auto_memory(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()> {
        let facts = self
            .auto_memory
            .lock()
            .await
            .analyze_and_store(&outcome.input, &outcome.output)
            .await?;
        if !facts.is_empty() {
            info!(count = facts.len(), "Auto-memory: stored facts");
        }
        Ok(())
    }

    async fn record_reflection(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()> {
        let task_summary = bounded_summary(&outcome.input, 100);
        let mut what_worked = vec![format!("Conversation turn #{}", outcome.turn)];
        let mut what_failed = Vec::new();
        let mut learned = Vec::new();
        let response_len = outcome.output.len();
        what_worked.push(if response_len > 500 {
            format!("Detailed response ({response_len} chars)")
        } else if response_len > 100 {
            format!("Concise response ({response_len} chars)")
        } else {
            format!("Brief response ({response_len} chars)")
        });
        let lower = outcome.output.to_lowercase();
        for indicator in [
            "error",
            "failed",
            "unable",
            "cannot",
            "couldn't",
            "sorry, i",
            "i don't know",
        ] {
            if lower.contains(indicator) {
                what_failed.push(format!("Response contains '{indicator}'"));
            }
        }
        for indicator in [
            "i learned",
            "i now understand",
            "i realize",
            "correction:",
            "actually,",
        ] {
            if lower.contains(indicator) {
                learned.push(format!("Self-correction detected: '{indicator}'"));
            }
        }
        let entry = self.reflector.reflect_conversation(
            &task_summary,
            ReflectionTrigger::TaskComplete,
            what_failed.is_empty(),
            what_worked,
            what_failed,
            learned,
        );
        let memory = self.episodic_memory.lock().await;
        memory.store_reflection(&entry)?;
        info!(id = %entry.id, task = %task_summary, "Chat reflection stored");
        if memory.reflection_count()? > 0 && memory.reflection_count()? % 10 == 0 {
            let recent = memory.recall_reflections(20)?;
            let summarizer = cognit::core::ExperienceSummarizer::new(self.clock.clone());
            if let Some(evolution) = summarizer.summarize(&recent) {
                memory.store_evolution_log(&evolution)?;
            }
        }
        Ok(())
    }

    async fn run_evolution(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()> {
        self.runtime
            .lock()
            .await
            .post_evolution(
                &bounded_summary(&outcome.input, 100),
                &outcome.output,
                outcome.completed_normally && !outcome.output.starts_with("error:"),
                outcome.tool_calls_made,
                outcome.tool_errors,
                outcome.elapsed_ms,
                outcome.iterations,
                &*self.evolution_pipeline,
            )
            .await
            .map(|_| ())
    }

    async fn persist_agora_commits(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()> {
        let commits = self
            .agora
            .changes_since(&outcome.session_id, outcome.agora_start_version)
            .await;
        let memory = self.recall_memory.lock().await;
        for commit in commits {
            memory.store(
                &outcome.session_id,
                "agora_commit",
                &serde_json::to_string(&commit)?,
                None,
            )?;
        }
        Ok(())
    }
}

fn bounded_summary(input: &str, max_chars: usize) -> String {
    let end = input
        .char_indices()
        .nth(max_chars)
        .map_or(input.len(), |(index, _)| index);
    if end < input.len() {
        format!("{}...", &input[..end])
    } else {
        input.to_owned()
    }
}
