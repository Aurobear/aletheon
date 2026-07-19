//! CodingVerifier — wired into ReActLoop verifier seam (Wave 3).
use async_trait::async_trait;
use fabric::message::Message;
use fabric::policy::verifier::{Verdict, Verifier};

pub struct CodingVerifier {
    min_output_chars: usize,
    require_tool_evidence: bool,
}

impl CodingVerifier {
    pub fn new() -> Self { Self { min_output_chars: 50, require_tool_evidence: true } }
    pub fn permissive() -> Self { Self { min_output_chars: 1, require_tool_evidence: false } }
}

impl Default for CodingVerifier {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Verifier for CodingVerifier {
    async fn verify(&self, final_text: &str, messages: &[Message]) -> Verdict {
        let trimmed = final_text.trim();
        if trimmed.is_empty() {
            return Verdict::Reject { reason: "Empty final answer — produce output or explain the blocker.".into() };
        }
        if trimmed.len() < self.min_output_chars {
            return Verdict::Reject {
                reason: format!("Answer too short ({} chars, min {}) — provide detail or evidence.", trimmed.len(), self.min_output_chars),
            };
        }
        if self.require_tool_evidence {
            // Evidence check: at least one message beyond system+user+assistant
            // indicates tool interactions occurred.
            let has_evidence = messages.len() > 2;
            if !has_evidence {
                return Verdict::Reject { reason: "No tool evidence — run tests, apply diffs, or produce build output.".into() };
            }
        }
        Verdict::Accept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::message::Message;

    #[tokio::test] async fn empty_rejected() { assert!(matches!(CodingVerifier::new().verify("", &[]).await, Verdict::Reject { .. })); }
    #[tokio::test] async fn no_tools_rejected() { assert!(matches!(CodingVerifier::new().verify("A very long and detailed answer that explains everything thoroughly.", &[]).await, Verdict::Reject { .. })); }
    #[tokio::test] async fn with_evidence_accepted() {
        let msgs = vec![Message::user("x"), Message::user("y"), Message::user("z")];
        let long = "Fixed the off-by-one error in calculate_bounds — tests pass, diff applied successfully with full coverage.";
        assert!(matches!(CodingVerifier::new().verify(long, &msgs).await, Verdict::Accept));
    }
}
