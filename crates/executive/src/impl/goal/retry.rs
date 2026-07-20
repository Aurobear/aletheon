//! Bounded retry and escalation policy for durable Goal attempts.
//!
//! This module only makes decisions. It never sleeps or invokes a runtime;
//! callers persist [`GoalWaitReason::Backoff`](fabric::GoalWaitReason::Backoff)
//! using their injected wall clock before scheduling a later attempt.

use fabric::{AttemptEvidence, CognitiveRole, FailureClass, RuntimeFailure, RuntimeId};

/// Per-role limits and exponential-backoff bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_worker_attempts: u32,
    pub max_reviewer_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_worker_attempts: 3,
            max_reviewer_attempts: 2,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 30_000,
        }
    }
}

/// One terminal policy choice after a completed attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    RetrySame {
        after_ms: u64,
        evidence: Vec<AttemptEvidence>,
    },
    Escalate {
        runtime_id: RuntimeId,
        evidence: Vec<AttemptEvidence>,
    },
    AwaitHuman {
        reason: String,
    },
    Fail {
        reason: String,
    },
    Cancel,
}

impl RetryPolicy {
    /// Decide what follows `attempt_count`, which includes the failed attempt.
    ///
    /// `escalation_runtime` is resolved by configuration before a worker call.
    /// Keeping runtime selection outside this policy prevents provider/model
    /// routing from leaking into supervision.
    pub fn decide(
        &self,
        role: CognitiveRole,
        attempt_count: u32,
        failure: &RuntimeFailure,
        escalation_runtime: Option<&RuntimeId>,
    ) -> RetryDecision {
        use FailureClass::*;

        if failure.class == Cancelled {
            return RetryDecision::Cancel;
        }

        if matches!(failure.class, PermissionDenied | ArchitectureViolation) {
            return RetryDecision::AwaitHuman {
                reason: bounded_reason(&failure.message),
            };
        }

        if matches!(failure.class, ProviderPermanent) {
            return RetryDecision::Fail {
                reason: bounded_reason(&failure.message),
            };
        }

        let max_attempts = match role {
            CognitiveRole::Reviewer | CognitiveRole::Verifier => self.max_reviewer_attempts,
            CognitiveRole::Worker | CognitiveRole::Debugger => self.max_worker_attempts,
        };

        if attempt_count >= max_attempts {
            return match role {
                CognitiveRole::Worker | CognitiveRole::Debugger => escalation_runtime
                    .cloned()
                    .map(|runtime_id| RetryDecision::Escalate {
                        runtime_id,
                        evidence: failure.evidence.clone(),
                    })
                    .unwrap_or_else(|| RetryDecision::AwaitHuman {
                        reason: "worker attempts exhausted without an escalation runtime".into(),
                    }),
                CognitiveRole::Reviewer | CognitiveRole::Verifier => RetryDecision::AwaitHuman {
                    reason: "reviewer attempts exhausted".into(),
                },
            };
        }

        if !failure.retryable
            && !matches!(
                failure.class,
                Compilation
                    | TestFailure
                    | MissingDependency
                    | InvalidAssumption
                    | ContextInsufficient
                    | RepeatedFailure
            )
        {
            return RetryDecision::Fail {
                reason: bounded_reason(&failure.message),
            };
        }

        RetryDecision::RetrySame {
            after_ms: self.backoff_ms(attempt_count),
            evidence: failure.evidence.clone(),
        }
    }

    fn backoff_ms(&self, attempt_count: u32) -> u64 {
        let exponent = attempt_count.saturating_sub(1).min(63);
        self.initial_backoff_ms
            .saturating_mul(1_u64 << exponent)
            .min(self.max_backoff_ms)
    }
}

fn bounded_reason(message: &str) -> String {
    const MAX_REASON_BYTES: usize = 512;
    let stored = RuntimeFailure {
        class: FailureClass::ProviderPermanent,
        message: message.into(),
        retryable: false,
        usage: Default::default(),
        evidence: vec![],
    }
    .bounded_for_persistence(MAX_REASON_BYTES);
    stored.message
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{AttemptUsage, FailureClass};

    fn failure(class: FailureClass, retryable: bool) -> RuntimeFailure {
        RuntimeFailure {
            class,
            message: format!("{class:?} token=secret"),
            retryable,
            usage: AttemptUsage::default(),
            evidence: vec![AttemptEvidence {
                kind: "diagnostic".into(),
                summary: format!("{class:?}"),
                content: "bounded compiler or test evidence".into(),
            }],
        }
    }

    #[test]
    fn every_failure_class_has_an_explicit_first_attempt_decision() {
        let policy = RetryPolicy::default();
        let reviewer = RuntimeId("reviewer".into());
        let cases = [
            (FailureClass::Compilation, true, "retry"),
            (FailureClass::TestFailure, true, "retry"),
            (FailureClass::PermissionDenied, false, "human"),
            (FailureClass::Timeout, true, "retry"),
            (FailureClass::MissingDependency, false, "retry"),
            (FailureClass::InvalidAssumption, false, "retry"),
            (FailureClass::ArchitectureViolation, false, "human"),
            (FailureClass::ToolFailure, true, "retry"),
            (FailureClass::ContextInsufficient, false, "retry"),
            (FailureClass::ProviderTransient, true, "retry"),
            (FailureClass::ProviderPermanent, false, "fail"),
            (FailureClass::Cancelled, false, "cancel"),
            (FailureClass::RepeatedFailure, false, "retry"),
        ];

        for (class, retryable, expected) in cases {
            let actual = policy.decide(
                CognitiveRole::Worker,
                1,
                &failure(class, retryable),
                Some(&reviewer),
            );
            let kind = match actual {
                RetryDecision::RetrySame { .. } => "retry",
                RetryDecision::Escalate { .. } => "escalate",
                RetryDecision::AwaitHuman { .. } => "human",
                RetryDecision::Fail { .. } => "fail",
                RetryDecision::Cancel => "cancel",
            };
            assert_eq!(kind, expected, "failure class {class:?}");
        }
    }

    #[test]
    fn backoff_is_exponential_and_bounded_without_sleeping() {
        let policy = RetryPolicy {
            initial_backoff_ms: 100,
            max_backoff_ms: 250,
            max_worker_attempts: 10,
            ..RetryPolicy::default()
        };
        let failure = failure(FailureClass::ProviderTransient, true);
        let delays: Vec<u64> = (1..=4)
            .map(
                |count| match policy.decide(CognitiveRole::Worker, count, &failure, None) {
                    RetryDecision::RetrySame { after_ms, .. } => after_ms,
                    other => panic!("unexpected decision: {other:?}"),
                },
            )
            .collect();
        assert_eq!(delays, vec![100, 200, 250, 250]);
    }

    #[test]
    fn third_worker_failure_escalates_with_evidence() {
        let policy = RetryPolicy::default();
        let reviewer = RuntimeId("reviewer-distinct".into());
        let failure = failure(FailureClass::TestFailure, true);
        let decision = policy.decide(CognitiveRole::Worker, 3, &failure, Some(&reviewer));
        assert_eq!(
            decision,
            RetryDecision::Escalate {
                runtime_id: reviewer,
                evidence: failure.evidence,
            }
        );
    }

    #[test]
    fn exhausted_reviewer_waits_for_human() {
        let policy = RetryPolicy::default();
        let decision = policy.decide(
            CognitiveRole::Reviewer,
            2,
            &failure(FailureClass::TestFailure, true),
            None,
        );
        assert!(matches!(decision, RetryDecision::AwaitHuman { .. }));
    }

    #[test]
    fn auth_policy_and_cancellation_never_retry() {
        let policy = RetryPolicy::default();
        for class in [
            FailureClass::PermissionDenied,
            FailureClass::ArchitectureViolation,
            FailureClass::Cancelled,
        ] {
            let decision = policy.decide(
                CognitiveRole::Worker,
                1,
                &failure(class, true),
                Some(&RuntimeId("reviewer".into())),
            );
            assert!(!matches!(decision, RetryDecision::RetrySame { .. }));
        }
    }

    #[test]
    fn non_retryable_tool_failure_fails_and_redacts_reason() {
        let decision = RetryPolicy::default().decide(
            CognitiveRole::Worker,
            1,
            &failure(FailureClass::ToolFailure, false),
            None,
        );
        let RetryDecision::Fail { reason } = decision else {
            panic!("expected fail")
        };
        assert!(!reason.contains("secret"));
    }
}
