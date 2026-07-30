//! Generic comparative capability harness. Every configured runtime receives the
//! exact same versioned task packet; the harness records observations without
//! embedding product identities or declaring a winner from incomplete evidence.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fabric::cognitive_workflow::{AgentTaskPacket, CognitiveRoleOutput};
use fabric::{
    AgentBudget, AgentContextFork, AgentControlPort, AgentId, AgentProfileId, AgentRunStatus,
    AgentSpawnRequest, AgentWaitRequest, OperationId, ProcessId, RuntimeId, WorkspacePolicy,
};
use serde::{Deserialize, Serialize};

use super::evaluation::{EvaluationProjectionRecord, EvaluationProjectionSink};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityRollupKey {
    pub runtime_id: String,
    pub profile_id: String,
    pub rubric_id: String,
    pub rubric_version: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityMetricAggregate {
    pub known_count: u64,
    pub total: u64,
}

impl CapabilityMetricAggregate {
    fn observe(&mut self, value: Option<u64>) {
        if let Some(value) = value {
            self.known_count += 1;
            self.total = self.total.saturating_add(value);
        }
    }
}

/// Optional materialized view rebuilt exclusively from immutable evaluation
/// receipt references. Usage dimensions stay separate rather than being folded
/// into the quality score.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReceiptRollup {
    pub receipt_count: u64,
    pub pass_count: u64,
    pub scores_millis: Vec<u32>,
    pub evidence_coverage_millis: CapabilityMetricAggregate,
    pub confidence_millis: CapabilityMetricAggregate,
    pub elapsed_ms: CapabilityMetricAggregate,
    pub inference_rounds: CapabilityMetricAggregate,
    pub provider_retries: CapabilityMetricAggregate,
    pub tool_calls: CapabilityMetricAggregate,
    pub tool_errors: CapabilityMetricAggregate,
    pub cumulative_input_tokens: CapabilityMetricAggregate,
    pub cumulative_output_tokens: CapabilityMetricAggregate,
    pub active_context_tokens: CapabilityMetricAggregate,
    pub cache_read_tokens: CapabilityMetricAggregate,
    pub cache_write_tokens: CapabilityMetricAggregate,
    #[serde(skip)]
    receipt_ids: HashSet<fabric::EvaluationReceiptId>,
}

pub struct CapabilityRollupProjectionSink {
    rollups: Mutex<HashMap<CapabilityRollupKey, CapabilityReceiptRollup>>,
    durable: Option<Mutex<rusqlite::Connection>>,
}

impl Default for CapabilityRollupProjectionSink {
    fn default() -> Self {
        Self {
            rollups: Mutex::new(HashMap::new()),
            durable: None,
        }
    }
}

impl CapabilityRollupProjectionSink {
    pub fn open(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let connection = rusqlite::Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS evaluation_rollup_inputs (
               receipt_id TEXT PRIMARY KEY NOT NULL,
               record_json TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL
             );",
        )?;
        let records = {
            let mut statement = connection.prepare(
                "SELECT record_json FROM evaluation_rollup_inputs
                 ORDER BY created_at_ms, receipt_id",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let sink = Self {
            rollups: Mutex::new(HashMap::new()),
            durable: Some(Mutex::new(connection)),
        };
        for encoded in records {
            let record = serde_json::from_str::<EvaluationProjectionRecord>(&encoded)?;
            sink.observe(&record);
        }
        Ok(sink)
    }

    pub fn snapshot(&self) -> HashMap<CapabilityRollupKey, CapabilityReceiptRollup> {
        self.rollups
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn observe(&self, record: &EvaluationProjectionRecord) {
        let key = CapabilityRollupKey {
            runtime_id: record.context.runtime_id.clone(),
            profile_id: record.context.profile_id.clone(),
            rubric_id: record.context.rubric_id.clone(),
            rubric_version: record.context.rubric_version,
        };
        let mut rollups = self
            .rollups
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let rollup = rollups.entry(key).or_default();
        if !rollup.receipt_ids.insert(record.receipt.receipt_id) {
            return;
        }
        rollup.receipt_count += 1;
        if matches!(
            record.receipt.decision,
            fabric::EvaluationDecision::ObservedPass | fabric::EvaluationDecision::Accepted
        ) {
            rollup.pass_count += 1;
        }
        if let Some(score) = record.receipt.weighted_total_millis {
            rollup.scores_millis.push(score);
            rollup.scores_millis.sort_unstable();
        }
        rollup
            .evidence_coverage_millis
            .observe(Some(u64::from(record.receipt.evidence_coverage_millis)));
        rollup
            .confidence_millis
            .observe(Some(u64::from(record.receipt.confidence_millis)));
        let metrics = &record.context.metrics;
        rollup.elapsed_ms.observe(metrics.elapsed_ms);
        rollup.inference_rounds.observe(metrics.inference_rounds);
        rollup.provider_retries.observe(metrics.provider_retries);
        rollup.tool_calls.observe(metrics.tool_calls);
        rollup.tool_errors.observe(metrics.tool_errors);
        rollup
            .cumulative_input_tokens
            .observe(metrics.cumulative_input_tokens);
        rollup
            .cumulative_output_tokens
            .observe(metrics.cumulative_output_tokens);
        rollup
            .active_context_tokens
            .observe(metrics.active_context_tokens);
        rollup.cache_read_tokens.observe(metrics.cache_read_tokens);
        rollup
            .cache_write_tokens
            .observe(metrics.cache_write_tokens);
    }
}

#[async_trait]
impl EvaluationProjectionSink for CapabilityRollupProjectionSink {
    fn name(&self) -> &'static str {
        "capability_rollup"
    }

    async fn project(&self, record: &EvaluationProjectionRecord) -> anyhow::Result<()> {
        if let Some(connection) = &self.durable {
            let encoded = serde_json::to_string(record)?;
            let inserted = connection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .execute(
                    "INSERT OR IGNORE INTO evaluation_rollup_inputs
                     (receipt_id, record_json, created_at_ms) VALUES (?1, ?2, ?3)",
                    rusqlite::params![
                        record.receipt.receipt_id.0.to_string(),
                        encoded,
                        record.receipt.created_at_ms
                    ],
                )?;
            if inserted == 0 {
                return Ok(());
            }
        }
        self.observe(record);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkTarget {
    pub runtime_id: RuntimeId,
    pub profile_id: AgentProfileId,
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityBenchmarkMetrics {
    pub cumulative_input_tokens: Option<u64>,
    pub cumulative_output_tokens: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub inference_rounds: Option<u64>,
    pub provider_retries: Option<u64>,
    pub tool_calls: Option<u64>,
    pub terminal_tool_results: Option<u64>,
    pub active_context_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityBenchmarkValidation {
    pub terminal_succeeded: bool,
    pub typed_output_valid: bool,
    pub required_evidence: Vec<String>,
    pub satisfied_evidence: Vec<String>,
}

impl CapabilityBenchmarkValidation {
    pub fn passed(&self) -> bool {
        self.terminal_succeeded
            && self.typed_output_valid
            && self.required_evidence == self.satisfied_evidence
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityBenchmarkObservation {
    pub runtime_id: RuntimeId,
    pub profile_id: AgentProfileId,
    pub task_packet_digest: String,
    pub process_id: ProcessId,
    pub operation_id: OperationId,
    pub terminal_status: AgentRunStatus,
    pub metrics: CapabilityBenchmarkMetrics,
    pub validation: CapabilityBenchmarkValidation,
    pub evidence_kinds: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityBenchmarkReport {
    pub task_packet_digest: String,
    pub evaluation_dimensions: Vec<String>,
    pub observations: Vec<CapabilityBenchmarkObservation>,
    /// False means the report is still useful raw evidence, but no comparative
    /// maturity or efficiency conclusion is justified.
    pub comparison_complete: bool,
    pub missing_dimensions: Vec<String>,
}

#[async_trait]
pub trait CapabilityBenchmarkRuntime: Send + Sync {
    async fn run(
        &self,
        target: &BenchmarkTarget,
        packet: AgentTaskPacket,
    ) -> anyhow::Result<CapabilityBenchmarkObservation>;
}

pub struct CapabilityBenchmarkHarness {
    executor: Arc<dyn CapabilityBenchmarkRuntime>,
}

impl CapabilityBenchmarkHarness {
    pub fn new(executor: Arc<dyn CapabilityBenchmarkRuntime>) -> Self {
        Self { executor }
    }

    pub async fn run(
        &self,
        packet: AgentTaskPacket,
        targets: Vec<BenchmarkTarget>,
    ) -> anyhow::Result<CapabilityBenchmarkReport> {
        packet.validate()?;
        anyhow::ensure!(
            targets.len() >= 2,
            "capability comparison requires at least two targets"
        );
        let mut unique = HashSet::new();
        anyhow::ensure!(
            targets
                .iter()
                .all(|target| unique.insert(target.runtime_id.clone())),
            "capability comparison target runtimes must be unique"
        );
        let digest = packet.digest()?;
        let mut observations = Vec::with_capacity(targets.len());
        // Sequential execution avoids turning the benchmark itself into a
        // provider-concurrency/backpressure confounder.
        for target in &targets {
            let observation = self.executor.run(target, packet.clone()).await?;
            anyhow::ensure!(
                observation.task_packet_digest == digest,
                "benchmark runtime observed a different task packet"
            );
            anyhow::ensure!(
                observation.runtime_id == target.runtime_id
                    && observation.profile_id == target.profile_id,
                "benchmark observation target identity mismatch"
            );
            observations.push(observation);
        }

        let dimensions = metric_dimensions();
        let missing_dimensions = dimensions
            .iter()
            .filter(|dimension| {
                observations
                    .iter()
                    .any(|observation| metric(&observation.metrics, dimension).is_none())
            })
            .cloned()
            .collect::<Vec<_>>();
        let comparison_complete = missing_dimensions.is_empty()
            && observations
                .iter()
                .all(|observation| observation.validation.passed());
        Ok(CapabilityBenchmarkReport {
            task_packet_digest: digest,
            evaluation_dimensions: dimensions,
            observations,
            comparison_complete,
            missing_dimensions,
        })
    }
}

#[derive(Clone)]
pub struct AgentControlBenchmarkRuntime {
    control: Arc<dyn AgentControlPort>,
    root_agent_id: AgentId,
    parent_agent_id: Option<AgentId>,
    parent_process_id: Option<ProcessId>,
    trusted_workspace: WorkspacePolicy,
    wait_timeout_ms: u64,
}

impl AgentControlBenchmarkRuntime {
    pub fn new(
        control: Arc<dyn AgentControlPort>,
        root_agent_id: AgentId,
        parent_agent_id: Option<AgentId>,
        parent_process_id: Option<ProcessId>,
        trusted_workspace: WorkspacePolicy,
        wait_timeout_ms: u64,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            wait_timeout_ms > 0,
            "benchmark wait timeout must be nonzero"
        );
        Ok(Self {
            control,
            root_agent_id,
            parent_agent_id,
            parent_process_id,
            trusted_workspace,
            wait_timeout_ms,
        })
    }
}

#[async_trait]
impl CapabilityBenchmarkRuntime for AgentControlBenchmarkRuntime {
    async fn run(
        &self,
        target: &BenchmarkTarget,
        packet: AgentTaskPacket,
    ) -> anyhow::Result<CapabilityBenchmarkObservation> {
        packet.validate()?;
        let digest = packet.digest()?;
        let packet_json = serde_json::to_string(&packet)?;
        let task = format!(
            "Execute this versioned cognitive task packet. Return only one JSON CognitiveRoleOutput object whose projection_id and workspace_version exactly match the packet; prose cannot satisfy validation.\n{packet_json}"
        );
        let budget = &packet.role_profile.budget;
        let handle = self
            .control
            .spawn(AgentSpawnRequest {
                root_agent_id: self.root_agent_id,
                parent_agent_id: self.parent_agent_id,
                parent_process_id: self.parent_process_id,
                profile_id: target.profile_id.clone(),
                runtime_id: target.runtime_id.clone(),
                trusted_workspace: Some(self.trusted_workspace.clone()),
                cognitive_binding: None,
                task,
                context: AgentContextFork::None,
                broadcast_refs: Vec::new(),
                allowed_tools: target.allowed_tools.clone(),
                budget: AgentBudget {
                    max_input_tokens: budget.max_input_tokens,
                    max_output_tokens: budget.max_output_tokens,
                    max_tool_calls: budget.max_tool_calls,
                    max_elapsed_ms: budget.max_elapsed_ms,
                    max_cost_usd: None,
                    max_depth: 1,
                },
                background_decls: Vec::new(),
            })
            .await
            .map_err(anyhow::Error::new)?;
        let snapshot = self
            .control
            .wait(AgentWaitRequest {
                caller_root_agent_id: self.root_agent_id,
                agent_id: handle.agent_id,
                timeout_ms: self.wait_timeout_ms.min(budget.max_elapsed_ms),
            })
            .await
            .map_err(anyhow::Error::new)?;
        anyhow::ensure!(
            snapshot.status.is_terminal(),
            "benchmark wait returned a non-terminal snapshot"
        );

        let mut typed_output_valid = false;
        let mut satisfied_evidence = Vec::new();
        let mut evidence_kinds = Vec::new();
        let metrics = if let Some(result) = &snapshot.result {
            let output = serde_json::from_str::<CognitiveRoleOutput>(&result.output).ok();
            typed_output_valid = output
                .as_ref()
                .is_some_and(|output| output.validate_for(&packet).is_ok());
            evidence_kinds = result
                .evidence
                .iter()
                .map(|evidence| evidence.kind.clone())
                .collect();
            if let Some(output) = output.filter(|_| typed_output_valid) {
                satisfied_evidence = packet
                    .expected_evidence
                    .iter()
                    .filter(|required| {
                        output.evidence_refs.contains(required)
                            || result.evidence.iter().any(|evidence| {
                                &evidence.kind == *required || &evidence.content == *required
                            })
                    })
                    .cloned()
                    .collect();
            }
            metrics_from_usage(&result.usage)
        } else {
            CapabilityBenchmarkMetrics::default()
        };
        let required_evidence = packet.expected_evidence.clone();
        Ok(CapabilityBenchmarkObservation {
            runtime_id: target.runtime_id.clone(),
            profile_id: target.profile_id.clone(),
            task_packet_digest: digest,
            process_id: handle.process_id,
            operation_id: handle.operation_id,
            terminal_status: snapshot.status,
            metrics,
            validation: CapabilityBenchmarkValidation {
                terminal_succeeded: snapshot.status == AgentRunStatus::Succeeded,
                typed_output_valid,
                required_evidence,
                satisfied_evidence,
            },
            evidence_kinds,
            error: snapshot.last_error,
        })
    }
}

fn metrics_from_usage(usage: &fabric::AttemptUsage) -> CapabilityBenchmarkMetrics {
    CapabilityBenchmarkMetrics {
        cumulative_input_tokens: Some(usage.input_tokens),
        cumulative_output_tokens: Some(usage.output_tokens),
        elapsed_ms: Some(usage.elapsed_ms),
        inference_rounds: usage.observability.inference_rounds,
        provider_retries: usage.observability.provider_retries,
        tool_calls: usage.observability.tool_calls,
        terminal_tool_results: usage.observability.terminal_tool_results,
        active_context_tokens: usage.observability.active_context_tokens,
        cache_read_tokens: usage.observability.cache_read_tokens,
        cache_write_tokens: usage.observability.cache_write_tokens,
    }
}

fn metric_dimensions() -> Vec<String> {
    [
        "cumulative_input_tokens",
        "cumulative_output_tokens",
        "elapsed_ms",
        "inference_rounds",
        "provider_retries",
        "tool_calls",
        "terminal_tool_results",
        "active_context_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn metric(metrics: &CapabilityBenchmarkMetrics, dimension: &str) -> Option<u64> {
    match dimension {
        "cumulative_input_tokens" => metrics.cumulative_input_tokens,
        "cumulative_output_tokens" => metrics.cumulative_output_tokens,
        "elapsed_ms" => metrics.elapsed_ms,
        "inference_rounds" => metrics.inference_rounds,
        "provider_retries" => metrics.provider_retries,
        "tool_calls" => metrics.tool_calls,
        "terminal_tool_results" => metrics.terminal_tool_results,
        "active_context_tokens" => metrics.active_context_tokens,
        "cache_read_tokens" => metrics.cache_read_tokens,
        "cache_write_tokens" => metrics.cache_write_tokens,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::cognitive_workflow::*;
    use tokio::sync::Mutex;

    struct ScriptedRuntime {
        digests: Mutex<Vec<String>>,
        missing_active_context_for: Option<RuntimeId>,
    }

    #[async_trait]
    impl CapabilityBenchmarkRuntime for ScriptedRuntime {
        async fn run(
            &self,
            target: &BenchmarkTarget,
            packet: AgentTaskPacket,
        ) -> anyhow::Result<CapabilityBenchmarkObservation> {
            let digest = packet.digest()?;
            self.digests.lock().await.push(digest.clone());
            let mut metrics = CapabilityBenchmarkMetrics {
                cumulative_input_tokens: Some(100),
                cumulative_output_tokens: Some(20),
                elapsed_ms: Some(10),
                inference_rounds: Some(2),
                provider_retries: Some(0),
                tool_calls: Some(1),
                terminal_tool_results: Some(1),
                active_context_tokens: Some(60),
                cache_read_tokens: Some(5),
                cache_write_tokens: Some(3),
            };
            if self.missing_active_context_for.as_ref() == Some(&target.runtime_id) {
                metrics.active_context_tokens = None;
            }
            Ok(CapabilityBenchmarkObservation {
                runtime_id: target.runtime_id.clone(),
                profile_id: target.profile_id.clone(),
                task_packet_digest: digest,
                process_id: ProcessId::new(),
                operation_id: OperationId::new(),
                terminal_status: AgentRunStatus::Succeeded,
                metrics,
                validation: CapabilityBenchmarkValidation {
                    terminal_succeeded: true,
                    typed_output_valid: true,
                    required_evidence: packet.expected_evidence.clone(),
                    satisfied_evidence: packet.expected_evidence,
                },
                evidence_kinds: vec!["terminal_validation".into()],
                error: None,
            })
        }
    }

    fn packet() -> AgentTaskPacket {
        let role_profile = CognitiveRoleProfile::canonical(CognitiveRole::Explorer);
        let task_id = CognitiveTaskNodeId("benchmark-task".into());
        AgentTaskPacket {
            schema_version: 1,
            task: CognitiveTaskNode {
                id: task_id.clone(),
                parent_id: None,
                objective: "inspect a bounded fixture and report grounded evidence".into(),
                role: CognitiveRole::Explorer,
                stage: CognitiveStage::Investigation,
                status: CognitiveTaskStatus::Pending,
                owner: None,
                role_profile: role_profile.reference.clone(),
                budget: role_profile.budget.clone(),
                dependencies: Vec::new(),
                acceptance_criteria: vec!["return terminal validation evidence".into()],
                workspace_scope: vec!["fixture".into()],
                required_artifact_kinds: vec![CognitiveArtifactKind::Investigation],
                artifact_refs: Vec::new(),
                unresolved_finding_ids: Vec::new(),
            },
            role_profile,
            project_instructions: vec!["read before claiming".into()],
            workspace_roots: vec!["fixture".into()],
            allowed_capabilities: Vec::new(),
            expected_evidence: vec!["terminal_validation".into()],
            acceptance_criteria: vec!["return terminal validation evidence".into()],
            selected_artifacts: Vec::new(),
            projection_receipt: AgoraProjectionReceipt {
                projection_id: uuid::Uuid::new_v4(),
                space: fabric::AgoraSpaceId("benchmark".into()),
                workspace_version: 1,
                task_node_id: task_id,
                role: CognitiveRole::Explorer,
                included_artifact_ids: Vec::new(),
                omitted_artifact_ids: Vec::new(),
            },
        }
    }

    fn targets() -> Vec<BenchmarkTarget> {
        ["runtime-a", "runtime-b"]
            .into_iter()
            .map(|id| BenchmarkTarget {
                runtime_id: RuntimeId(id.into()),
                profile_id: AgentProfileId("benchmark-profile".into()),
                allowed_tools: vec!["file_read".into()],
            })
            .collect()
    }

    #[tokio::test]
    async fn every_runtime_receives_the_identical_typed_packet() {
        let runtime = Arc::new(ScriptedRuntime {
            digests: Mutex::new(Vec::new()),
            missing_active_context_for: None,
        });
        let report = CapabilityBenchmarkHarness::new(runtime.clone())
            .run(packet(), targets())
            .await
            .unwrap();

        let observed = runtime.digests.lock().await;
        assert_eq!(observed.len(), 2);
        assert!(observed
            .iter()
            .all(|digest| digest == &report.task_packet_digest));
        assert!(report.comparison_complete);
        assert!(report.missing_dimensions.is_empty());
    }

    #[tokio::test]
    async fn unavailable_dimension_makes_comparison_explicitly_incomplete() {
        let runtime = Arc::new(ScriptedRuntime {
            digests: Mutex::new(Vec::new()),
            missing_active_context_for: Some(RuntimeId("runtime-b".into())),
        });
        let report = CapabilityBenchmarkHarness::new(runtime)
            .run(packet(), targets())
            .await
            .unwrap();

        assert!(!report.comparison_complete);
        assert_eq!(report.missing_dimensions, vec!["active_context_tokens"]);
        assert_eq!(report.observations.len(), 2);
    }

    #[tokio::test]
    async fn capability_rollup_uses_receipt_quality_and_separate_usage_dimensions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollups.db");
        let sink = CapabilityRollupProjectionSink::open(&path).unwrap();
        let receipt_id = fabric::EvaluationReceiptId::new();
        let record = EvaluationProjectionRecord {
            receipt: fabric::EvaluationReceiptRef {
                schema_version: fabric::EVALUATION_SCHEMA_V1,
                receipt_id,
                contract_id: fabric::EvaluationContractId::new(),
                subject_kind: "turn".into(),
                subject_id: fabric::TurnId::new().0.to_string(),
                decision: fabric::EvaluationDecision::ObservedPass,
                weighted_total_millis: Some(82_000),
                evidence_coverage_millis: 750,
                confidence_millis: 880,
                failed_gates: vec![],
                created_at_ms: 1,
            },
            context: super::super::evaluation::EvaluationProjectionContext {
                session_id: "session".into(),
                runtime_id: "native".into(),
                profile_id: "code-agent".into(),
                rubric_id: "coding-v2".into(),
                rubric_version: 2,
                process_id: ProcessId::new(),
                metrics: super::super::evaluation::EvaluationProjectionMetrics {
                    elapsed_ms: Some(100),
                    inference_rounds: Some(3),
                    provider_retries: Some(1),
                    tool_calls: Some(5),
                    ..Default::default()
                },
            },
        };

        sink.project(&record).await.unwrap();
        sink.project(&record).await.unwrap();

        drop(sink);
        let sink = CapabilityRollupProjectionSink::open(path).unwrap();
        let rollups = sink.snapshot();
        let rollup = rollups.values().next().unwrap();
        assert_eq!(rollup.receipt_count, 1, "receipt replay is idempotent");
        assert_eq!(rollup.pass_count, 1);
        assert_eq!(rollup.scores_millis, vec![82_000]);
        assert_eq!(rollup.inference_rounds.total, 3);
        assert_eq!(rollup.provider_retries.total, 1);
        assert_eq!(rollup.tool_calls.total, 5);
        assert_eq!(rollup.elapsed_ms.total, 100);
        assert_eq!(rollup.confidence_millis.total, 880);
    }
}
