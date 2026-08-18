//! Mapping from Cognit/inference failures to typed Application Turn outcomes.

pub fn classify_runtime_turn_failure(
    error: &anyhow::Error,
) -> (contracts::TurnStop, Option<contracts::TurnFailure>) {
    let (stop, kind, retryable) = match error.downcast_ref::<cognit::CognitError>() {
        Some(cognitive) => match cognitive.kind() {
            cognit::CognitErrorKind::Cancelled => return (contracts::TurnStop::Cancelled, None),
            cognit::CognitErrorKind::ContextOverflow => (
                contracts::TurnStop::Failed,
                contracts::TurnFailureKind::ContextOverflow,
                false,
            ),
            cognit::CognitErrorKind::TransientProvider => (
                contracts::TurnStop::Failed,
                contracts::TurnFailureKind::ProviderTransient,
                true,
            ),
            cognit::CognitErrorKind::TerminalProvider => (
                contracts::TurnStop::Failed,
                contracts::TurnFailureKind::ProviderPermanent,
                false,
            ),
            cognit::CognitErrorKind::TerminalRuntime => (
                contracts::TurnStop::Failed,
                contracts::TurnFailureKind::Runtime,
                false,
            ),
        },
        None => match error.downcast_ref::<cognit::inference::InferenceFailure>() {
            Some(inference) => match inference.kind {
                cognit::inference::InferenceFailureKind::Transient => (
                    contracts::TurnStop::Failed,
                    contracts::TurnFailureKind::ProviderTransient,
                    true,
                ),
                cognit::inference::InferenceFailureKind::ContextOverflow => (
                    contracts::TurnStop::Failed,
                    contracts::TurnFailureKind::ContextOverflow,
                    false,
                ),
                cognit::inference::InferenceFailureKind::Terminal => (
                    contracts::TurnStop::Failed,
                    contracts::TurnFailureKind::ProviderPermanent,
                    false,
                ),
            },
            None => (
                contracts::TurnStop::Failed,
                contracts::TurnFailureKind::Runtime,
                false,
            ),
        },
    };
    (
        stop,
        Some(contracts::TurnFailure {
            kind,
            message: error.to_string(),
            retryable,
        }),
    )
}

pub struct NormalizedCognitiveResult {
    pub result: contracts::TurnResult,
    pub execution_returned: bool,
    pub runtime_faults: Vec<String>,
}

pub fn normalize(result: anyhow::Result<contracts::TurnResult>) -> NormalizedCognitiveResult {
    match result {
        Ok(result) => NormalizedCognitiveResult {
            result,
            execution_returned: true,
            runtime_faults: Vec::new(),
        },
        Err(error) => {
            let runtime_faults = vec![error.to_string()];
            let (stop, failure) = classify_runtime_turn_failure(&error);
            let output = if stop == contracts::TurnStop::Cancelled {
                "Cancelled by user. The cancelled turn objective is closed.".to_string()
            } else {
                format!("error: {error}")
            };
            NormalizedCognitiveResult {
                result: contracts::TurnResult {
                    output,
                    stop,
                    failure,
                    usage: Default::default(),
                    metrics: contracts::TurnMetrics::default(),
                },
                execution_returned: false,
                runtime_faults,
            }
        }
    }
}
