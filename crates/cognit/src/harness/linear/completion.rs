use std::future::Future;

use fabric::message::Message;
use fabric::policy::verifier::Verdict;
use tracing::warn;

use super::ReActLoop;

pub(super) enum FinalizationDecision {
    ContinueWithInterjections {
        assistant_text: Option<String>,
        interjections: Vec<String>,
    },
    ContinueAfterRejection,
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
        let interjections = drain_interjections().await?;
        if !interjections.is_empty() {
            return Ok(FinalizationDecision::ContinueWithInterjections {
                assistant_text: (!final_text.is_empty()).then_some(final_text),
                interjections,
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

        Ok(FinalizationDecision::Accept { final_text })
    }
}
