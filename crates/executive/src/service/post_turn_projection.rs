//! Non-blocking projections of an already-settled turn.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use fabric::hook::{HookContext, HookPoint};

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
    runtime: Arc<tokio::sync::Mutex<crate::core::orchestrator::AletheonExecutive>>,
    evolution: Arc<dyn metacog::MetacogService>,
}

type HookProjectionFn =
    dyn Fn(HookContext) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;

pub struct PostTurnProjectionResources {
    pub corpus: Arc<dyn corpus::CorpusService>,
    pub executive: Arc<tokio::sync::Mutex<crate::core::orchestrator::AletheonExecutive>>,
    pub evolution: Arc<dyn metacog::MetacogService>,
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
            runtime: resources.executive,
            evolution: resources.evolution,
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
                self.evolution.as_ref(),
            )
            .await
            .map(|_| ())
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
