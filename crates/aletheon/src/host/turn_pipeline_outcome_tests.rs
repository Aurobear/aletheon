use super::{TurnPipelineOutcome, TurnPipelineRejection};

#[test]
fn rejection_messages_preserve_the_former_json_error_text() {
    assert_eq!(
        TurnPipelineRejection::IntentDeniedBySelfField {
            reason: "read denied".into()
        }
        .message(),
        "Intent denied by SelfField: read denied"
    );
    assert_eq!(
        TurnPipelineRejection::SelfFieldReviewFailed {
            reason: "provider down".into()
        }
        .message(),
        "SelfField review failed (fail-closed): provider down"
    );
    assert_eq!(
        TurnPipelineRejection::HookBlocked {
            reason: "guard rejected".into()
        }
        .message(),
        "Blocked by hook: guard rejected"
    );
}

#[test]
fn rejection_variants_are_distinct_domain_values() {
    let denied = TurnPipelineRejection::IntentDeniedBySelfField {
        reason: "same".into(),
    };
    let hook = TurnPipelineRejection::HookBlocked {
        reason: "same".into(),
    };
    assert_ne!(denied, hook);
}

#[test]
fn outcome_carries_the_typed_rejection() {
    let outcome = TurnPipelineOutcome::Rejected(TurnPipelineRejection::HookBlocked {
        reason: "block".into(),
    });
    assert!(matches!(
        outcome,
        TurnPipelineOutcome::Rejected(TurnPipelineRejection::HookBlocked { reason })
            if reason == "block"
    ));
}
