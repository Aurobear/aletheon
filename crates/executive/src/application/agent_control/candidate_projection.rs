//! Projection of child runtime events into typed, bounded C01 candidates.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentId, AgentResult, AttemptEvidence, Clock,
    ContentId, Evidence, Hypothesis, MonoDeadline, ProcessId, SalienceVector, VisibilityScope,
    WorkspaceAttribution, WorkspaceCandidate, WorkspaceContent, WorkspaceObservation,
    WorkspaceProvenance, WorkspaceReflection, WORKSPACE_SCHEMA_V1,
};
use parking_lot::Mutex;
use uuid::Uuid;

use super::{AgentEventSink, AgentRuntimeEvent, AgentRuntimeInput};
use crate::application::conscious_core_ports::{CandidateSubmission, CandidateSubmissionReceipt};

const CANDIDATE_NAMESPACE: Uuid = Uuid::from_u128(0xc00b_9e90_0dc5_4d80_86b4_8aaf_9917_a6d2);
const MAX_INLINE_BYTES: usize = 4 * 1024;
const MAX_EXPORTED_EVIDENCE: usize = 16;
const MAX_EXPORTED_ARTIFACTS: usize = 16;
const TTL_MS: u64 = 60_000;

/// Narrow C01 admission boundary used by Agent runtimes. Implementations admit
/// candidates to a pool; they must not select, commit, or broadcast them.
#[async_trait]
pub trait AgentCandidateSubmissionPort: Send + Sync {
    async fn submit_agent_candidate(
        &self,
        submission: CandidateSubmission,
        recipient: ProcessId,
        root: ProcessId,
    ) -> anyhow::Result<CandidateSubmissionReceipt>;
}

pub struct AgentCandidateProjector {
    input: AgentRuntimeInput,
    clock: Arc<dyn Clock>,
}

impl AgentCandidateProjector {
    pub fn new(input: AgentRuntimeInput, clock: Arc<dyn Clock>) -> Self {
        Self { input, clock }
    }

    pub fn project(
        &self,
        event: &AgentRuntimeEvent,
    ) -> Result<Vec<CandidateSubmission>, AgentControlError> {
        let (agent, process, operation) = event_ids(event);
        if agent != self.input.handle.agent_id
            || process != self.input.handle.process_id
            || operation != self.input.handle.operation_id
        {
            return Err(AgentControlError::invalid(
                "runtime event identity differs from its durable Agent run",
            ));
        }
        let (kind, contents) = match event {
            AgentRuntimeEvent::Started { .. } => (
                "started",
                vec![(
                    WorkspaceContent::Observation(self.observation("child runtime started")),
                    false,
                )],
            ),
            AgentRuntimeEvent::Progress { summary, .. } => (
                "progress",
                vec![(
                    WorkspaceContent::Observation(self.observation(&bound(summary))),
                    false,
                )],
            ),
            AgentRuntimeEvent::Tool { name, is_error, .. } => (
                "tool",
                vec![(
                    WorkspaceContent::Observation(WorkspaceObservation {
                        what: format!(
                            "tool {} {}",
                            bound(name),
                            if *is_error { "failed" } else { "completed" }
                        ),
                        source: "child-tool-event".into(),
                        data: serde_json::json!({ "tool": bound(name), "is_error": is_error }),
                        attribution: WorkspaceAttribution::ChildAgent {
                            process: self.input.handle.process_id,
                        },
                    }),
                    false,
                )],
            ),
            AgentRuntimeEvent::Terminal { status, result, .. } => {
                let mut values = Vec::new();
                if let Some(result) = result {
                    for evidence in &result.evidence {
                        values.push((self.evidence_content(evidence), is_exportable(evidence)));
                    }
                    values.push((WorkspaceContent::AgentResult(bound_result(result)), true));
                } else {
                    values.push((
                        WorkspaceContent::Reflection(WorkspaceReflection {
                            findings: vec![format!("child runtime ended with {status:?}")],
                            confidence: 1.0,
                        }),
                        false,
                    ));
                }
                ("terminal", values)
            }
        };

        contents
            .into_iter()
            .enumerate()
            .map(|(index, (content, exportable))| self.submission(kind, index, content, exportable))
            .collect()
    }

    fn observation(&self, what: &str) -> WorkspaceObservation {
        WorkspaceObservation {
            what: what.into(),
            source: "child-runtime".into(),
            data: serde_json::json!({ "agent_id": self.input.handle.agent_id }),
            attribution: WorkspaceAttribution::ChildAgent {
                process: self.input.handle.process_id,
            },
        }
    }

    fn evidence_content(&self, evidence: &AttemptEvidence) -> WorkspaceContent {
        let kind = evidence.kind.trim_start_matches("exportable:");
        match kind {
            "hypothesis" => WorkspaceContent::Hypothesis(Hypothesis {
                id: format!(
                    "agent:{}:{}",
                    self.input.handle.agent_id.0, evidence.summary
                ),
                statement: bound(&evidence.content),
                confidence: 0.5,
                evidence_ids: vec![],
            }),
            "criticism" => WorkspaceContent::Reflection(WorkspaceReflection {
                findings: vec![bound(&evidence.content)],
                confidence: 0.7,
            }),
            _ => WorkspaceContent::Evidence(Evidence {
                id: format!(
                    "agent:{}:{}",
                    self.input.handle.agent_id.0, evidence.summary
                ),
                source: format!("child-agent:{kind}"),
                content: bound(&evidence.content),
                weight: 1.0,
            }),
        }
    }

    fn submission(
        &self,
        kind: &str,
        index: usize,
        content: WorkspaceContent,
        exportable: bool,
    ) -> Result<CandidateSubmission, AgentControlError> {
        let event_ref = format!(
            "agent-runtime:{}:{}:{kind}",
            self.input.handle.agent_id.0, self.input.handle.operation_id.0
        );
        let now = self.clock.mono_now();
        let content_key = serde_json::to_vec(&content)
            .map_err(|error| AgentControlError::invalid(error.to_string()))?;
        let candidate = WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: ContentId(Uuid::new_v5(
                &CANDIDATE_NAMESPACE,
                [
                    format!("{event_ref}:{index}:{}:", self.input.workspace_id.0).as_bytes(),
                    &content_key,
                ]
                .concat()
                .as_slice(),
            )),
            space: if exportable {
                self.input.root_workspace_id.clone()
            } else {
                self.input.workspace_id.clone()
            },
            source: self.input.handle.process_id,
            turn: None,
            content,
            confidence: 0.8,
            salience: agent_salience(),
            provenance: WorkspaceProvenance {
                producer: self.input.handle.process_id,
                operation: Some(self.input.handle.operation_id),
                source_refs: std::iter::once(event_ref.clone())
                    .chain(std::iter::once(format!(
                        "agent:{}",
                        self.input.handle.agent_id.0
                    )))
                    .chain(self.input.request.broadcast_refs.iter().map(|reference| {
                        format!("broadcast:{}:{}", reference.space.0, reference.epoch.0)
                    }))
                    .collect(),
                observed_at: self.clock.wall_now(),
            },
            visibility: if exportable {
                VisibilityScope::AgentTree {
                    root: self.input.root_process_id,
                }
            } else {
                VisibilityScope::PrivateProcess {
                    process: self.input.handle.process_id,
                }
            },
            dependencies: self
                .input
                .request
                .broadcast_refs
                .iter()
                .map(|reference| reference.content_id)
                .collect(),
            created_at: now,
            expires_at: Some(MonoDeadline::after(now, TTL_MS)),
        };
        candidate
            .validate()
            .map_err(|error| AgentControlError::invalid(error.to_string()))?;
        Ok(CandidateSubmission {
            candidate,
            cause: crate::application::conscious_core_ports::CandidateCause::AgentRuntimeEvent {
                agent_id: self.input.handle.agent_id,
                operation_id: self.input.handle.operation_id,
                event_kind: kind.into(),
            },
        })
    }
}

pub struct ProjectingAgentEventSink {
    downstream: Arc<dyn AgentEventSink>,
    port: Arc<dyn AgentCandidateSubmissionPort>,
    projector: AgentCandidateProjector,
    error: Mutex<Option<AgentControlError>>,
}

impl ProjectingAgentEventSink {
    pub fn new(
        downstream: Arc<dyn AgentEventSink>,
        port: Arc<dyn AgentCandidateSubmissionPort>,
        input: AgentRuntimeInput,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            downstream,
            port,
            projector: AgentCandidateProjector::new(input, clock),
            error: Mutex::new(None),
        }
    }

    pub fn take_error(&self) -> Option<AgentControlError> {
        self.error.lock().take()
    }
}

#[async_trait]
impl AgentEventSink for ProjectingAgentEventSink {
    async fn emit(&self, event: AgentRuntimeEvent) {
        if self.error.lock().is_none() {
            match self.projector.project(&event) {
                Ok(submissions) => {
                    for submission in submissions {
                        if let Err(error) = self
                            .port
                            .submit_agent_candidate(
                                submission,
                                self.projector.input.handle.process_id,
                                self.projector.input.root_process_id,
                            )
                            .await
                        {
                            *self.error.lock() = Some(AgentControlError {
                                kind: AgentControlErrorKind::Runtime,
                                message: error.to_string(),
                            });
                            break;
                        }
                    }
                }
                Err(error) => *self.error.lock() = Some(error),
            }
        }
        self.downstream.emit(event).await;
    }
}

fn is_exportable(evidence: &AttemptEvidence) -> bool {
    evidence.kind.starts_with("exportable:")
}

fn bound_result(result: &AgentResult) -> AgentResult {
    AgentResult {
        output: if result.output.len() <= MAX_INLINE_BYTES {
            result.output.clone()
        } else if result.artifacts.is_empty() {
            "[large child output omitted: no validated artifact reference supplied]".into()
        } else {
            "[large child output omitted; use validated artifact references]".into()
        },
        usage: result.usage.clone(),
        evidence: result
            .evidence
            .iter()
            .filter(|item| is_exportable(item))
            .take(MAX_EXPORTED_EVIDENCE)
            .map(|item| AttemptEvidence {
                kind: bound(item.kind.trim_start_matches("exportable:")),
                summary: bound_to(&item.summary, 512),
                content: bound(&item.content),
            })
            .collect(),
        artifacts: result
            .artifacts
            .iter()
            .filter(|artifact| artifact.validate().is_ok())
            .take(MAX_EXPORTED_ARTIFACTS)
            .cloned()
            .collect(),
    }
}

fn bound(value: &str) -> String {
    bound_to(value, MAX_INLINE_BYTES)
}

fn bound_to(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

fn event_ids(event: &AgentRuntimeEvent) -> (AgentId, ProcessId, fabric::OperationId) {
    match event {
        AgentRuntimeEvent::Started {
            agent_id,
            process_id,
            operation_id,
        }
        | AgentRuntimeEvent::Progress {
            agent_id,
            process_id,
            operation_id,
            ..
        }
        | AgentRuntimeEvent::Tool {
            agent_id,
            process_id,
            operation_id,
            ..
        }
        | AgentRuntimeEvent::Terminal {
            agent_id,
            process_id,
            operation_id,
            ..
        } => (*agent_id, *process_id, *operation_id),
    }
}

fn agent_salience() -> SalienceVector {
    SalienceVector {
        urgency: 0.3,
        goal_relevance: 0.8,
        self_relevance: 0.4,
        novelty: 0.6,
        confidence: 0.8,
        prediction_error: 0.0,
        affect_intensity: 0.0,
        social_relevance: 0.2,
    }
}
