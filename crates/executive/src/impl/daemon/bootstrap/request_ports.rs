//! Private concrete adapters for request use-case ports.

use std::path::Path;
use std::sync::Arc;

use dasein::SelfField;
use mnemosyne::episodic::EpisodicMemory;
use tokio::sync::Mutex;

use fabric::{Subsystem, SubsystemContext};

use crate::core::orchestrator::AletheonExecutive;
use crate::service::request_use_cases::{
    CareWeight, ExecutiveRuntimePort, ReflectionMemoryPort, ReflectionStats, RuntimeStatus,
    SelfStatus, SelfStatusPort, SupplementalMemoryStatus, SupplementalMemoryStatusPort,
};

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
