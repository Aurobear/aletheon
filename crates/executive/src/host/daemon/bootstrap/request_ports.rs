//! Private concrete adapters for request use-case ports.

use std::path::Path;
use std::sync::Arc;

use dasein::SelfField;
use mnemosyne::runtime::EpisodicMemory;
use tokio::sync::Mutex;

use fabric::{Subsystem, SubsystemContext};

use crate::application::admin_service::{AdminRuntimePort, ModeChange};
use crate::application::post_turn_projection::{PostTurnOutcome, PostTurnRuntimePort};
use crate::application::request_use_cases::{
    CareWeight, ExecutiveRuntimePort, ReflectionEnginePort, ReflectionMemoryPort, ReflectionStats,
    RetentionAdminPort, RuntimeStatus, SelfStatus, SelfStatusPort, SupplementalMemoryStatus,
    SupplementalMemoryStatusPort,
};
use crate::application::turn_runtime_ports::{SelfPolicyPort, TurnConfigPort};
use crate::composition::config::GrokHardeningConfig;
use crate::core::orchestrator::AletheonExecutive;

pub(super) async fn initialize_self_field(
    self_field: &mut SelfField,
    data_dir: &Path,
) -> anyhow::Result<()> {
    self_field
        .init(&SubsystemContext {
            name: "self_field".into(),
            working_dir: data_dir.to_path_buf(),
            config: serde_json::Value::Null,
            bus: None,
        })
        .await
}

pub(super) fn retention_admin_port(
    repository: Arc<mnemosyne::RetentionRepository>,
) -> Arc<dyn RetentionAdminPort> {
    Arc::new(RetentionAdminAdapter { repository })
}

pub(super) fn reflection_engine_port(
    reflector: cognit::core::reflector::Reflector,
) -> Arc<dyn ReflectionEnginePort> {
    Arc::new(ReflectionEngineAdapter { reflector })
}

pub(super) fn admin_runtime_port(
    runtime: Arc<Mutex<AletheonExecutive>>,
) -> Arc<dyn AdminRuntimePort> {
    Arc::new(ExecutiveDomainAdapter { executive: runtime })
}

pub(super) fn post_turn_runtime_port(
    runtime: Arc<Mutex<AletheonExecutive>>,
    evolution: Arc<dyn metacog::MetacogService>,
    self_field: Arc<Mutex<SelfField>>,
    clock: Arc<dyn fabric::Clock>,
) -> Arc<dyn PostTurnRuntimePort> {
    Arc::new(PostTurnDomainAdapter {
        executive: runtime,
        evolution,
        self_field,
        mood_fallback: Arc::new(metacog::MetaCognition::new(None, clock)),
    })
}

pub(super) struct RequestFacadePorts {
    pub(super) runtime_port: Arc<dyn ExecutiveRuntimePort>,
    pub(super) reflections: Arc<dyn ReflectionMemoryPort>,
    pub(super) self_status: Arc<dyn SelfStatusPort>,
    pub(super) supplemental: Arc<dyn SupplementalMemoryStatusPort>,
}

impl RequestFacadePorts {
    pub(super) fn new(
        runtime: Arc<Mutex<AletheonExecutive>>,
        episodic: Arc<Mutex<EpisodicMemory>>,
        self_field: Arc<Mutex<SelfField>>,
        supplemental: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
        _grok_hardening: GrokHardeningConfig,
    ) -> Self {
        Self {
            runtime_port: Arc::new(ExecutiveRuntimeAdapter { executive: runtime }),
            reflections: Arc::new(ReflectionMemoryAdapter { episodic }),
            self_status: Arc::new(SelfStatusAdapter { self_field }),
            supplemental: Arc::new(SupplementalMemoryStatusAdapter {
                health: supplemental,
            }),
        }
    }
}

pub(super) struct TurnRuntimeFacadePorts {
    pub(super) self_policy: Arc<dyn SelfPolicyPort>,
    pub(super) config: Arc<dyn TurnConfigPort>,
}

impl TurnRuntimeFacadePorts {
    pub(super) fn new(
        runtime: Arc<Mutex<AletheonExecutive>>,
        self_field: Arc<Mutex<SelfField>>,
    ) -> Self {
        Self {
            self_policy: Arc::new(SelfPolicyAdapter { field: self_field }),
            config: Arc::new(TurnConfigAdapter {
                config_source: runtime,
            }),
        }
    }
}

struct ExecutiveRuntimeAdapter {
    executive: Arc<Mutex<AletheonExecutive>>,
}

struct ExecutiveDomainAdapter {
    executive: Arc<Mutex<AletheonExecutive>>,
}

struct PostTurnDomainAdapter {
    executive: Arc<Mutex<AletheonExecutive>>,
    evolution: Arc<dyn metacog::MetacogService>,
    self_field: Arc<Mutex<SelfField>>,
    mood_fallback: Arc<metacog::MetaCognition>,
}

#[async_trait::async_trait]
impl AdminRuntimePort for ExecutiveDomainAdapter {
    async fn request_interrupt(&self, reason: fabric::ui_event::InterruptReason) {
        self.executive.lock().await.interrupt_flag().request(reason);
    }

    async fn switch_mode(&self, mode: fabric::CollaborationMode) -> ModeChange {
        let mut runtime = self.executive.lock().await;
        let old = runtime.mode_router().current_mode();
        runtime.mode_router_mut().set_mode(mode);
        ModeChange { old, new: mode }
    }
}

#[async_trait::async_trait]
impl PostTurnRuntimePort for PostTurnDomainAdapter {
    async fn post_evolution(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()> {
        let summary = self
            .executive
            .lock()
            .await
            .post_evolution(
                &crate::application::post_turn_projection::bounded_summary(&outcome.input, 100),
                &outcome.output,
                outcome.completed_normally && !outcome.output.starts_with("error:"),
                outcome.tool_calls_made,
                outcome.tool_errors,
                outcome.elapsed_ms,
                outcome.iterations,
                self.evolution.as_ref(),
            )
            .await?;

        // Evidence-backed proposals always win. The mood adapter is only the
        // transition fallback while the reflection/proposal pipeline has not
        // produced a governed candidate for this turn.
        if evidence_proposal_has_priority(summary.as_ref()) {
            return Ok(());
        }

        let Some(context) = self.self_field.lock().await.dasein_context() else {
            tracing::debug!(
                turn = outcome.turn,
                "Mood evolution fallback skipped because Dasein context is unavailable"
            );
            return Ok(());
        };
        let decision = self.mood_fallback.decide(&context, outcome.turn);
        match decision {
            metacog::EvolutionAction::TriggerEvolution { intents } => {
                let triggered = self
                    .executive
                    .lock()
                    .await
                    .post_mood_fallback(&intents, self.evolution.as_ref())
                    .await?;
                tracing::info!(
                    turn = outcome.turn,
                    intent_count = intents.len(),
                    triggered,
                    "Dasein mood evolution fallback evaluated"
                );
            }
            metacog::EvolutionAction::Observe => {
                tracing::debug!(
                    turn = outcome.turn,
                    "Dasein mood evolution fallback observed"
                );
            }
            metacog::EvolutionAction::AdjustDasein { parameter, value } => {
                tracing::info!(
                    turn = outcome.turn,
                    %parameter,
                    value,
                    "Dasein mood evolution fallback requested bounded adjustment"
                );
            }
            metacog::EvolutionAction::InjectReflection { content } => {
                tracing::info!(
                    turn = outcome.turn,
                    reflection = %content,
                    "Dasein mood evolution fallback emitted reflection"
                );
            }
        }
        Ok(())
    }
}

fn evidence_proposal_has_priority(
    summary: Option<&crate::core::evolution_coordinator::EvolutionSummary>,
) -> bool {
    summary.is_some_and(|summary| summary.evolution_triggered)
}

#[async_trait::async_trait]
impl ExecutiveRuntimePort for ExecutiveRuntimeAdapter {
    async fn status(&self) -> RuntimeStatus {
        let runtime = self.executive.lock().await;
        RuntimeStatus {
            session_id: runtime.config().session_id.clone(),
            iteration: runtime.iteration(),
        }
    }

    async fn request_interrupt(&self, reason: fabric::ui_event::InterruptReason) {
        self.executive.lock().await.interrupt_flag().request(reason);
    }
}

struct ReflectionMemoryAdapter {
    episodic: Arc<Mutex<EpisodicMemory>>,
}

struct ReflectionEngineAdapter {
    reflector: cognit::core::reflector::Reflector,
}

impl ReflectionEnginePort for ReflectionEngineAdapter {
    fn reflect_conversation(
        &self,
        conversation: &str,
        trigger: fabric::ReflectionTrigger,
        succeeded: bool,
        what_worked: Vec<String>,
        what_failed: Vec<String>,
        learned: Vec<String>,
    ) -> fabric::ReflectionEntry {
        self.reflector.reflect_conversation(
            conversation,
            trigger,
            succeeded,
            what_worked,
            what_failed,
            learned,
        )
    }
}

#[async_trait::async_trait]
impl ReflectionMemoryPort for ReflectionMemoryAdapter {
    async fn stats(&self) -> ReflectionStats {
        let episodic = self.episodic.lock().await;
        ReflectionStats {
            reflection_count: episodic.reflection_count().unwrap_or(0),
            evolution_count: episodic.evolution_log_count().unwrap_or(0),
        }
    }

    async fn recall_reflections(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<fabric::ReflectionEntry>> {
        self.episodic.lock().await.recall_reflections(limit)
    }

    async fn store_reflection(&self, entry: &fabric::ReflectionEntry) -> anyhow::Result<()> {
        self.episodic.lock().await.store_reflection(entry)
    }

    async fn recall_evolution_logs(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<fabric::EvolutionLogEntry>> {
        self.episodic.lock().await.recall_evolution_logs(limit)
    }
}

struct SelfStatusAdapter {
    self_field: Arc<Mutex<SelfField>>,
}

#[async_trait::async_trait]
impl SelfStatusPort for SelfStatusAdapter {
    async fn status(&self) -> SelfStatus {
        let self_field = self.self_field.lock().await;
        SelfStatus {
            care_weights: self_field
                .care()
                .all_cares()
                .into_iter()
                .map(|care| CareWeight {
                    topic: care.topic,
                    weight: care.weight,
                })
                .collect(),
            boundary_rules: self_field.boundary().rule_count(),
            boundary_immutable: self_field.boundary().immutable_rule_count(),
            attention_focus: self_field
                .current_attention_focus()
                .map(|focus| focus.topic)
                .unwrap_or_default(),
        }
    }
}

struct SupplementalMemoryStatusAdapter {
    health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
}

struct RetentionAdminAdapter {
    repository: Arc<mnemosyne::RetentionRepository>,
}

impl RetentionAdminPort for RetentionAdminAdapter {
    fn compact(
        &self,
        owner: &str,
        now_ms: i64,
        policy: &mnemosyne::RetentionCompactionPolicy,
    ) -> anyhow::Result<mnemosyne::RetentionCompactionReport> {
        mnemosyne::RetentionCompactor::new(&self.repository).run(owner, now_ms, policy)
    }
}

impl SupplementalMemoryStatusPort for SupplementalMemoryStatusAdapter {
    fn status(&self) -> SupplementalMemoryStatus {
        let health = self.health.lock().unwrap();
        SupplementalMemoryStatus {
            enabled: health.supplemental_enabled,
            degraded: health.degraded,
            queue_depth: health.queue_depth,
        }
    }
}

struct SelfPolicyAdapter {
    field: Arc<Mutex<SelfField>>,
}

#[async_trait::async_trait]
impl SelfPolicyPort for SelfPolicyAdapter {
    async fn review(
        &self,
        intent: &fabric::Intent,
        context: &fabric::Context,
    ) -> anyhow::Result<fabric::Verdict> {
        use fabric::SelfFieldOps;
        self.field.lock().await.review(intent, context).await
    }

    async fn narrate(&self, event: &str, reason: &str) {
        use fabric::SelfFieldOps;
        let _ = self.field.lock().await.narrate(event, reason).await;
    }

    async fn coordinate(&self, turn: usize, output: &str, status: fabric::dasein::OutcomeStatus) {
        let field = self.field.lock().await;
        if let Some(dasein) = field.dasein() {
            match dasein.record_outcome(output, status, "turn-pipeline").await {
                Ok(receipt) => tracing::info!(
                    turn,
                    version = receipt.current_version.0,
                    "Dasein outcome accepted"
                ),
                Err(error) => tracing::warn!(turn, %error, "Dasein outcome rejected"),
            }
        }
    }

    fn dasein_context_provider(&self) -> Arc<dyn Fn() -> Option<String> + Send + Sync> {
        let field = self.field.clone();
        Arc::new(move || {
            field
                .try_lock()
                .ok()
                .and_then(|field| field.dasein_prompt_injection())
        })
    }
}

struct TurnConfigAdapter {
    config_source: Arc<Mutex<AletheonExecutive>>,
}

#[async_trait::async_trait]
impl TurnConfigPort for TurnConfigAdapter {
    async fn config(&self) -> crate::composition::config::ExecutiveConfig {
        self.config_source.lock().await.config().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::evidence_proposal_has_priority;
    use crate::core::evolution_coordinator::EvolutionSummary;

    fn summary(evolution_triggered: bool) -> EvolutionSummary {
        EvolutionSummary {
            reflected: true,
            reflection_id: Some("reflection".into()),
            evolution_triggered,
            verification_receipts: Vec::new(),
            lineage_entries_added: 0,
            awareness_entries: Vec::new(),
        }
    }

    #[test]
    fn evidence_backed_proposal_suppresses_mood_fallback() {
        assert!(evidence_proposal_has_priority(Some(&summary(true))));
        assert!(!evidence_proposal_has_priority(Some(&summary(false))));
        assert!(!evidence_proposal_has_priority(None));
    }
}
