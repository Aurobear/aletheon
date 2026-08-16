//! Private concrete adapters for request use-case ports.

use std::path::Path;
use std::sync::Arc;

use dasein::bridge::loop_detector::{LoopDecision, LoopDecisionPort};
use dasein::bridge::policy::{PolicyDecision, PolicyDecisionPort};
use dasein::{SelfField, SelfFieldConfig};
use mnemosyne::runtime::EpisodicMemory;
use tokio::sync::Mutex;

use ::contracts::{Subsystem, SubsystemContext};

use crate::config::GrokHardeningConfig;
use crate::wiring::application::admin_service::{AdminRuntimePort, ModeChange};
use crate::wiring::application::post_turn_projection::{PostTurnOutcome, PostTurnRuntimePort};
use crate::wiring::application::request_use_cases::{
    CareWeight, CognitiveRuntimePort, ReflectionEnginePort, ReflectionMemoryPort, ReflectionStats,
    RetentionAdminPort, RuntimeStatus, SelfStatus, SelfStatusPort, SupplementalMemoryStatus,
    SupplementalMemoryStatusPort,
};
use crate::wiring::application::turn_runtime_ports::{SelfPolicyPort, TurnConfigPort};
use crate::wiring::cognitive_runtime::AletheonCognitiveRuntime;

pub(super) async fn initialize_self_field(
    self_field: &mut SelfField,
    data_dir: &Path,
) -> anyhow::Result<()> {
    self_field
        .init(&SubsystemContext {
            name: "self_field".into(),
            working_dir: data_dir.to_path_buf(),
            config: serde_json::Value::Null,
        })
        .await
}

pub(super) fn compose_self_field(
    data_dir: &Path,
    clock: Arc<dyn ::contracts::Clock>,
    conscious_context: Arc<agora::ConsciousContextSlot>,
) -> SelfField {
    let (policy_decisions, loop_decisions) = corpus_security_ports();
    SelfField::new(SelfFieldConfig {
        db_path: Some(data_dir.join("self_field.db")),
        clock: Some(clock),
        conscious_context: Some(conscious_context),
        policy_decisions: Some(policy_decisions),
        loop_decisions: Some(loop_decisions),
        ..Default::default()
    })
}

pub(super) fn corpus_security_ports() -> (Arc<dyn PolicyDecisionPort>, Arc<dyn LoopDecisionPort>) {
    (
        Arc::new(CorpusPolicyDecisionAdapter {
            engine: corpus::security::policy::PolicyEngine::with_defaults(),
        }),
        Arc::new(CorpusLoopDecisionAdapter {
            detector: std::sync::Mutex::new(corpus::security::loop_detector::LoopDetector::new(
                corpus::security::loop_detector::LoopDetectorConfig::default(),
            )),
        }),
    )
}

struct CorpusPolicyDecisionAdapter {
    engine: corpus::security::policy::PolicyEngine,
}

impl PolicyDecisionPort for CorpusPolicyDecisionAdapter {
    fn check(&self, tool_name: &str, input: &serde_json::Value) -> PolicyDecision {
        match self.engine.check(tool_name, input) {
            corpus::security::policy::PolicyVerdict::Allow => PolicyDecision::Allow,
            corpus::security::policy::PolicyVerdict::Deny { reason } => {
                PolicyDecision::Deny { reason }
            }
            corpus::security::policy::PolicyVerdict::RequireApproval { reason } => {
                PolicyDecision::RequireApproval { reason }
            }
        }
    }
}

struct CorpusLoopDecisionAdapter {
    detector: std::sync::Mutex<corpus::security::loop_detector::LoopDetector>,
}

impl LoopDecisionPort for CorpusLoopDecisionAdapter {
    fn on_new_turn(&self, turn_id: &str) {
        self.detector
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .on_new_turn(turn_id);
    }

    fn pre_check(&self, tool_name: &str, args: &serde_json::Value, turn_id: &str) -> LoopDecision {
        match self
            .detector
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pre_check(tool_name, args, turn_id)
        {
            corpus::security::loop_detector::LoopVerdict::Allow => LoopDecision::Allow,
            corpus::security::loop_detector::LoopVerdict::Warn { reason } => {
                LoopDecision::Warn { reason }
            }
            corpus::security::loop_detector::LoopVerdict::Block { reason, suggestion } => {
                LoopDecision::Block { reason, suggestion }
            }
            corpus::security::loop_detector::LoopVerdict::Escalate { reason } => {
                LoopDecision::Escalate { reason }
            }
            corpus::security::loop_detector::LoopVerdict::InterruptTurn {
                reason,
                consecutive_blocks,
            } => LoopDecision::InterruptTurn {
                reason,
                consecutive_blocks,
            },
        }
    }

    fn post_check(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        result: &::contracts::tool::ToolResult,
        turn_id: &str,
    ) {
        self.detector
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .post_check(tool_name, args, result, turn_id);
    }

    fn end_turn(&self, turn_id: &str) {
        self.detector
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .end_turn(turn_id);
    }
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
    runtime: Arc<Mutex<AletheonCognitiveRuntime>>,
) -> Arc<dyn AdminRuntimePort> {
    Arc::new(CognitiveDomainAdapter { runtime: runtime })
}

pub(super) fn post_turn_runtime_port(
    runtime: Arc<Mutex<AletheonCognitiveRuntime>>,
    evolution: Arc<dyn metacog::MetacogService>,
    self_field: Arc<Mutex<SelfField>>,
    clock: Arc<dyn ::contracts::Clock>,
    proposer: Arc<crate::wiring::composition::evolution_proposer::GovernedEvolutionProposer>,
) -> Arc<dyn PostTurnRuntimePort> {
    Arc::new(PostTurnDomainAdapter {
        runtime: runtime,
        evolution,
        self_field,
        mood_fallback: Arc::new(metacog::MetaCognition::new(None, clock)),
        proposer,
    })
}

pub(super) struct RequestFacadePorts {
    pub(super) runtime_port: Arc<dyn CognitiveRuntimePort>,
    pub(super) reflections: Arc<dyn ReflectionMemoryPort>,
    pub(super) self_status: Arc<dyn SelfStatusPort>,
    pub(super) supplemental: Arc<dyn SupplementalMemoryStatusPort>,
}

impl RequestFacadePorts {
    pub(super) fn new(
        runtime: Arc<Mutex<AletheonCognitiveRuntime>>,
        episodic: Arc<Mutex<EpisodicMemory>>,
        self_field: Arc<Mutex<SelfField>>,
        supplemental: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
        _grok_hardening: GrokHardeningConfig,
    ) -> Self {
        Self {
            runtime_port: Arc::new(CognitiveRuntimeAdapter { runtime: runtime }),
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
        runtime: Arc<Mutex<AletheonCognitiveRuntime>>,
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

struct CognitiveRuntimeAdapter {
    runtime: Arc<Mutex<AletheonCognitiveRuntime>>,
}

struct CognitiveDomainAdapter {
    runtime: Arc<Mutex<AletheonCognitiveRuntime>>,
}

struct PostTurnDomainAdapter {
    runtime: Arc<Mutex<AletheonCognitiveRuntime>>,
    evolution: Arc<dyn metacog::MetacogService>,
    self_field: Arc<Mutex<SelfField>>,
    mood_fallback: Arc<metacog::MetaCognition>,
    proposer: Arc<crate::wiring::composition::evolution_proposer::GovernedEvolutionProposer>,
}

#[async_trait::async_trait]
impl AdminRuntimePort for CognitiveDomainAdapter {
    async fn request_interrupt(&self, reason: application::turn_control::InterruptReason) {
        self.runtime.lock().await.interrupt_flag().request(reason);
    }

    async fn switch_mode(&self, mode: application::turn_control::CollaborationMode) -> ModeChange {
        let mut runtime = self.runtime.lock().await;
        let old = runtime.mode_router().current_mode();
        runtime.mode_router_mut().set_mode(mode);
        ModeChange { old, new: mode }
    }
}

#[async_trait::async_trait]
impl PostTurnRuntimePort for PostTurnDomainAdapter {
    async fn post_evolution(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()> {
        let summary = self
            .runtime
            .lock()
            .await
            .post_evolution(
                &crate::wiring::application::post_turn_projection::bounded_summary(
                    &outcome.input,
                    100,
                ),
                &outcome.output,
                outcome.completed_normally && !outcome.output.starts_with("error:"),
                outcome.tool_calls_made,
                outcome.tool_errors,
                outcome.elapsed_ms,
                outcome.iterations,
                self.evolution.as_ref(),
            )
            .await?;

        if let Some(summary) = summary.as_ref() {
            for receipt in &summary.verification_receipts {
                match self.proposer.propose(
                    &outcome.session_id,
                    outcome.principal_id.clone(),
                    receipt,
                ) {
                    Ok(Some(approval)) => tracing::info!(
                        approval_id = %approval.id,
                        mutation_id = %receipt.mutation_id,
                        "governed evolution approval proposed"
                    ),
                    Ok(None) => tracing::info!(
                        mutation_id = %receipt.mutation_id,
                        "evolution candidate parked because durable A/B evidence is insufficient"
                    ),
                    Err(error) => tracing::warn!(%error, mutation_id = %receipt.mutation_id,
                        "failed to persist governed evolution proposal"),
                }
            }
        }

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
                    .runtime
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
    summary: Option<&crate::wiring::evolution_coordinator::EvolutionSummary>,
) -> bool {
    summary.is_some_and(|summary| summary.evolution_triggered)
}

#[async_trait::async_trait]
impl CognitiveRuntimePort for CognitiveRuntimeAdapter {
    async fn status(&self) -> RuntimeStatus {
        let runtime = self.runtime.lock().await;
        RuntimeStatus {
            session_id: runtime.config().session_id.clone(),
            iteration: runtime.iteration(),
        }
    }

    async fn request_interrupt(&self, reason: application::turn_control::InterruptReason) {
        self.runtime.lock().await.interrupt_flag().request(reason);
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
        trigger: cognit::domain::ReflectionTrigger,
        succeeded: bool,
        what_worked: Vec<String>,
        what_failed: Vec<String>,
        learned: Vec<String>,
    ) -> cognit::domain::ReflectionEntry {
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
    ) -> anyhow::Result<Vec<cognit::domain::ReflectionEntry>> {
        self.episodic.lock().await.recall_reflections(limit)
    }

    async fn store_reflection(
        &self,
        entry: &cognit::domain::ReflectionEntry,
    ) -> anyhow::Result<()> {
        self.episodic.lock().await.store_reflection(entry)
    }

    async fn recall_evolution_logs(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<cognit::domain::EvolutionLogEntry>> {
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
        let health = self.health.lock().unwrap_or_else(|e| e.into_inner());
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
        intent: &dasein::Intent,
        context: &::contracts::Context,
    ) -> anyhow::Result<dasein::Verdict> {
        use dasein::SelfFieldOps;
        self.field.lock().await.review(intent, context).await
    }

    async fn narrate(&self, event: &str, reason: &str) {
        use dasein::SelfFieldOps;
        let _ = self.field.lock().await.narrate(event, reason).await;
    }

    async fn coordinate(
        &self,
        turn: usize,
        output: &str,
        status: ::contracts::dasein::OutcomeStatus,
    ) {
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
    config_source: Arc<Mutex<AletheonCognitiveRuntime>>,
}

#[async_trait::async_trait]
impl TurnConfigPort for TurnConfigAdapter {
    async fn config(&self) -> crate::config::CognitiveRuntimeConfig {
        self.config_source.lock().await.config().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{corpus_security_ports, evidence_proposal_has_priority};
    use crate::wiring::evolution_coordinator::EvolutionSummary;
    use dasein::bridge::policy::PolicyDecision;

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

    #[test]
    fn injected_policy_port_preserves_corpus_default_verdicts() {
        let direct = corpus::security::policy::PolicyEngine::with_defaults();
        let (injected, _) = corpus_security_ports();
        for (tool, input) in [
            ("file_read", serde_json::json!({})),
            ("bash_exec", serde_json::json!({"command": "rm -rf /tmp/x"})),
            ("mkfs /dev/test", serde_json::json!({})),
        ] {
            let expected = direct.check(tool, &input);
            let actual = injected.check(tool, &input);
            assert!(matches!(
                (expected, actual),
                (
                    corpus::security::policy::PolicyVerdict::Allow,
                    PolicyDecision::Allow
                ) | (
                    corpus::security::policy::PolicyVerdict::Deny { .. },
                    PolicyDecision::Deny { .. }
                ) | (
                    corpus::security::policy::PolicyVerdict::RequireApproval { .. },
                    PolicyDecision::RequireApproval { .. }
                )
            ));
        }
    }
}
