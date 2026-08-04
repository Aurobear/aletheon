use std::future::Future;

use fabric::message::Message;
use fabric::policy::verifier::Verdict;
use tracing::warn;

use crate::core::{
    grounded_completion_outcomes, ClaimEvidenceAuditor, CompletionGateMode, ProgressAuditor,
};

use super::ReActLoop;

pub(super) enum FinalizationDecision {
    ContinueWithInterjections {
        assistant_text: Option<String>,
        interjections: Vec<String>,
    },
    ContinueAfterRejection,
    Incomplete {
        message: String,
    },
    Accept {
        final_text: String,
    },
}

impl ReActLoop {
    pub(super) async fn finalize_candidate<D, DFut>(
        &mut self,
        final_text: String,
        drain_interjections: &D,
    ) -> anyhow::Result<FinalizationDecision>
    where
        D: Fn() -> DFut,
        DFut: Future<Output = anyhow::Result<Vec<String>>>,
    {
        let mut accepted_outcomes = Vec::new();
        let mut rejected = None;
        let interjections = drain_interjections().await?;
        if !interjections.is_empty() {
            return Ok(FinalizationDecision::ContinueWithInterjections {
                assistant_text: (!final_text.is_empty()).then_some(final_text),
                interjections,
            });
        }

        if let Some(state) = self.cognitive_state.as_mut() {
            state.completion_attempts = state.completion_attempts.saturating_add(1);
            let decision = ProgressAuditor.audit(state, &self.evidence_ledger);
            let claim_audit = ClaimEvidenceAuditor.audit(state, &self.evidence_ledger);
            if !decision.is_complete() {
                if self.completion_gate_mode == CompletionGateMode::Enforce {
                    let outcomes = grounded_completion_outcomes(state, &claim_audit, false);
                    let recovery = decision.recovery_message().unwrap_or_else(|| {
                        "[cognitive_completion_rejected]\nRequired evidence is missing.".into()
                    });
                    rejected = Some((outcomes, recovery, state.completion_attempts));
                }
                warn!(decision = ?decision, "cognitive completion gate would reject in shadow mode");
            }
            if decision.is_complete() {
                accepted_outcomes = grounded_completion_outcomes(state, &claim_audit, true);
            }
            self.latest_completion_audit = Some(decision);
        }

        if let Some((outcomes, recovery, attempts)) = rejected {
            self.publish_grounded_outcomes(outcomes).await?;
            if attempts <= self.max_completion_retries {
                self.messages.push(Message::assistant(&final_text));
                self.messages.push(Message::user(recovery));
                return Ok(FinalizationDecision::ContinueAfterRejection);
            }
            return Ok(FinalizationDecision::Incomplete {
                message: format!(
                    "Task incomplete after {attempts} completion attempts.\n{recovery}"
                ),
            });
        }

        if let Some(verifier) = self.verifier.clone() {
            if self.verify_attempts < self.max_verify_attempts {
                if let Verdict::Reject { reason } =
                    verifier.verify(&final_text, &self.messages).await
                {
                    self.verify_attempts += 1;
                    self.messages.push(Message::assistant(&final_text));
                    self.messages.push(Message::user(format!(
                        "[verification] Your previous answer was rejected: {reason}\n\
                         Please correct it and provide a better final answer."
                    )));
                    warn!(
                        reason = reason.as_str(),
                        "verifier rejected final answer; retrying"
                    );
                    return Ok(FinalizationDecision::ContinueAfterRejection);
                }
            }
        }

        self.publish_grounded_outcomes(accepted_outcomes).await?;

        Ok(FinalizationDecision::Accept { final_text })
    }

    async fn publish_grounded_outcomes(
        &mut self,
        outcomes: Vec<fabric::cognitive_workflow::GroundedCognitiveOutcome>,
    ) -> anyhow::Result<()> {
        let Some(sink) = &self.grounded_outcome_sink else {
            return Ok(());
        };
        for outcome in outcomes {
            if let Err(error) = sink.publish(outcome).await {
                // Grounded outcomes are post-decision observations.  Dasein is
                // deliberately not an authority over deterministic completion.
                warn!(%error, "failed to publish grounded cognitive outcome");
            }
        }
        Ok(())
    }
}
