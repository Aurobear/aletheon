//! CodingCompletionVerifier — independent evidence-based completion gating (Wave 3).
//! Consumers: Wave 3 Pi adapter, Wave 5 profile benchmark gate.

use runtime_api::receipt::{CompletionStatus, RuntimeReceipt};

/// Decision after examining the runtime receipt and independent evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    Unverified,
    Blocked,
    NeedsRetry(Vec<String>),
}

pub struct CodingCompletionVerifier;

impl CodingCompletionVerifier {
    pub fn new() -> Self { Self }
}

pub fn verify(receipt: &RuntimeReceipt) -> Verdict {
    if receipt.status == CompletionStatus::SucceededVerified {
        Verdict::Verified
    } else if receipt.status == CompletionStatus::SucceededUnverified {
        if receipt.workspace_diff.is_some() && !receipt.output.is_empty() {
            Verdict::Verified
        } else {
            Verdict::Unverified
        }
    } else {
        Verdict::Blocked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_api::receipt::RuntimeUsage;

    #[test]
    fn verified_when_runtime_confirms() {
        let r = RuntimeReceipt {
            status: CompletionStatus::SucceededVerified,
            output: "done".into(),
            usage: RuntimeUsage { tokens_in: 100, tokens_out: 50, elapsed_ms: 1000 },
            workspace_diff: Some("+fn fix() {}".into()),
            errors: vec![],
        };
        assert_eq!(verify(&r), Verdict::Verified);
    }

    #[test]
    fn unverified_without_diff() {
        let r = RuntimeReceipt {
            status: CompletionStatus::SucceededUnverified,
            output: "done".into(),
            usage: RuntimeUsage { tokens_in: 100, tokens_out: 50, elapsed_ms: 1000 },
            workspace_diff: None,
            errors: vec![],
        };
        assert_eq!(verify(&r), Verdict::Unverified);
    }
}
