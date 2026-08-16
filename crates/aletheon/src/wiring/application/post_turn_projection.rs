//! Non-blocking projections of an already-settled turn.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use ::contracts::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, MessageId, NamespaceId, SchemaId,
};
use async_trait::async_trait;
use corpus::hook::{HookContext, HookPoint};
use runtime::{
    EventId, EventIdentity, EventPayload, EventSpine, EventTreeId, EventVisibility,
    UnsequencedEvent,
};
use uuid::Uuid;

use super::evaluation::{EvaluationProjectionRecord, EvaluationProjectionSink};

const EVALUATION_PROJECTION_EVENT_NAMESPACE: Uuid =
    Uuid::from_u128(0x73c8ae38_0cd8_49cf_86cc_761237c1a8ee);

#[derive(Clone, Debug)]
pub struct PostTurnOutcome {
    pub session_id: String,
    pub principal_id: ::contracts::PrincipalId,
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
    runtime: Arc<dyn PostTurnRuntimePort>,
}

type HookProjectionFn =
    dyn Fn(HookContext) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;

pub struct PostTurnProjectionResources {
    pub corpus: Arc<dyn corpus::CorpusService>,
    pub runtime: Arc<dyn PostTurnRuntimePort>,
}

/// Durable, idempotent observation boundary shared by Goal and AgentControl.
/// Domain-specific wrappers select the target name and add no scoring logic.
pub struct DurableDomainEvaluationSink {
    domain: &'static str,
    spine: Arc<dyn EventSpine>,
}

impl DurableDomainEvaluationSink {
    pub fn new(domain: &'static str, spine: Arc<dyn EventSpine>) -> Self {
        Self { domain, spine }
    }
}

#[async_trait]
impl EvaluationProjectionSink for DurableDomainEvaluationSink {
    fn name(&self) -> &'static str {
        self.domain
    }

    async fn project(&self, record: &EvaluationProjectionRecord) -> anyhow::Result<()> {
        let session_id = if record.context.session_id.trim().is_empty() {
            format!("evaluation:{}", record.receipt.subject_id)
        } else {
            record.context.session_id.clone()
        };
        let event_key = format!("{}:{}", self.domain, record.receipt.receipt_id.0);
        let event_id = EventId(Uuid::new_v5(
            &EVALUATION_PROJECTION_EVENT_NAMESPACE,
            event_key.as_bytes(),
        ));
        let payload = serde_json::json!({
            "kind": format!("evaluation.{}.observed", self.domain),
            "receipt": record.receipt,
            "correlation": record.context,
            "retry_replan_evidence": record.receipt.failed_gates,
        });
        let mut envelope = EnvelopeV2::new(
            SchemaId::from(SchemaId::EVENT_EVALUATION_OBSERVED_V1),
            EnvelopeV2Target("evaluation-projection".into()),
            EnvelopeV2Target(format!("{}:{}", self.domain, record.receipt.subject_id)),
            EnvelopeV2Delivery::Direct,
            NamespaceId(format!("evaluation:{}", self.domain)),
            payload.clone(),
        );
        envelope.id = MessageId(event_id.0);
        self.spine.append(UnsequencedEvent {
            tree_id: EventTreeId::for_root_session(&session_id),
            event_id,
            parent: None,
            identity: EventIdentity {
                root_session_id: session_id.clone(),
                session_id,
                agent_id: None,
            },
            envelope,
            visibility: EventVisibility::Control,
            payload: EventPayload::Inline { value: payload },
        })?;
        Ok(())
    }
}

#[async_trait]
pub trait PostTurnRuntimePort: Send + Sync {
    async fn post_evolution(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()>;
}

impl ProductionPostTurnProjection {
    pub fn new(resources: PostTurnProjectionResources) -> Self {
        let corpus = resources.corpus;
        let run_hook: Arc<HookProjectionFn> = Arc::new(move |context| {
            let corpus = corpus.clone();
            Box::pin(async move {
                corpus.execute_hook(&context).await;
            })
        });
        Self {
            run_hook,
            runtime: resources.runtime,
        }
    }
}

#[async_trait]
impl PostTurnProjection for ProductionPostTurnProjection {
    async fn project(&self, outcome: PostTurnOutcome) -> anyhow::Result<()> {
        let mut failures = Vec::new();
        self.run_post_turn_hook(&outcome).await;

        if let Err(error) = self.run_evolution(&outcome).await {
            failures.push(format!("evolution: {error}"));
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

    async fn run_evolution(&self, outcome: &PostTurnOutcome) -> anyhow::Result<()> {
        self.runtime.post_evolution(outcome).await
    }
}

pub fn bounded_summary(input: &str, max_chars: usize) -> String {
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
