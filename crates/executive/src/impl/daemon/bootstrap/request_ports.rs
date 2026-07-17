//! Private concrete adapters for request use-case ports.

use std::path::Path;
use std::sync::Arc;

use dasein::SelfField;
use mnemosyne::episodic::EpisodicMemory;
use tokio::sync::Mutex;

use fabric::{Subsystem, SubsystemContext};

use crate::core::orchestrator::AletheonExecutive;
use crate::service::request_use_cases::{
    CareWeight, ExecutiveRuntimePort, ReflectionEnginePort, ReflectionMemoryPort, ReflectionStats,
    RetentionAdminPort, RuntimeStatus, SelfStatus, SelfStatusPort, SupplementalMemoryStatus,
    SupplementalMemoryStatusPort,
};
use crate::service::turn_runtime_ports::{SelfPolicyPort, TurnConfigPort};

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
                .attention()
                .current_focus()
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
    async fn config(&self) -> crate::core::config::ExecutiveConfig {
        self.config_source.lock().await.config().clone()
    }
}
