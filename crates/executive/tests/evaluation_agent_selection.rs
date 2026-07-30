use executive::application::capability_benchmark::CapabilityRollupProjectionSink;
use executive::application::evaluation::{
    EvaluationProjectionContext, EvaluationProjectionMetrics, EvaluationProjectionRecord,
    EvaluationProjectionSink,
};
use fabric::{
    AgentBudget, AgentDelegationAuthority, EvaluationContractId, EvaluationDecision,
    EvaluationReceiptId, EvaluationReceiptRef, ProcessId, WorkspacePolicy, EVALUATION_SCHEMA_V1,
};

fn record(runtime: &str, passed: bool, score: u32, index: i64) -> EvaluationProjectionRecord {
    EvaluationProjectionRecord {
        receipt: EvaluationReceiptRef {
            schema_version: EVALUATION_SCHEMA_V1,
            receipt_id: EvaluationReceiptId::new(),
            contract_id: EvaluationContractId::new(),
            subject_kind: "agent_run".into(),
            subject_id: format!("agent-{index}"),
            decision: if passed {
                EvaluationDecision::ObservedPass
            } else {
                EvaluationDecision::ObservedFail
            },
            weighted_total_millis: Some(score),
            evidence_coverage_millis: 900,
            confidence_millis: 900,
            failed_gates: if passed { vec![] } else { vec!["tests".into()] },
            created_at_ms: index,
        },
        context: EvaluationProjectionContext {
            session_id: "session".into(),
            runtime_id: runtime.into(),
            profile_id: "code-agent".into(),
            effective_model_id: "provider/model".into(),
            model_display_name: "model".into(),
            workspace_boundary_sha256: "workspace".into(),
            verification_selection_sha256: "checks".into(),
            rubric_id: "coding-v2".into(),
            rubric_version: 2,
            process_id: ProcessId::new(),
            metrics: EvaluationProjectionMetrics::default(),
        },
    }
}

#[tokio::test]
async fn host_prefers_evaluated_runtime_without_creating_authority() {
    let sink = CapabilityRollupProjectionSink::default();
    sink.project(&record("runtime-a", false, 30_000, 1))
        .await
        .unwrap();
    sink.project(&record("runtime-b", true, 80_000, 2))
        .await
        .unwrap();
    assert_eq!(
        sink.preferred_runtime("code-agent", ["runtime-a", "runtime-b"]),
        Some("runtime-b".into())
    );
    assert_eq!(
        sink.preferred_runtime("other-profile", ["runtime-a", "runtime-b"]),
        None
    );

    let budget = AgentBudget {
        max_input_tokens: 10,
        max_output_tokens: 10,
        max_tool_calls: 10,
        max_elapsed_ms: 10,
        max_cost_usd: None,
        max_depth: 2,
    };
    let parent = AgentDelegationAuthority::new(
        Some(WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap()),
        vec!["read".into()],
        budget.clone(),
    );
    let requested = AgentDelegationAuthority::new(
        Some(
            WorkspacePolicy::from_resolved_roots("/tmp/child".into(), vec!["/var/tmp".into()])
                .unwrap(),
        ),
        vec!["read".into(), "shell".into()],
        AgentBudget {
            max_input_tokens: 20,
            ..budget
        },
    );
    let (effective, _) = parent.attenuate(&requested).unwrap();
    assert!(parent.covers(&effective));
    assert_eq!(effective.allowed_tools, vec!["read"]);
    assert_eq!(
        effective.workspace.unwrap().writable_roots(),
        &[std::path::PathBuf::from("/tmp/child")]
    );
}

#[tokio::test]
async fn replay_does_not_bias_selection() {
    let sink = CapabilityRollupProjectionSink::default();
    let same = record("runtime-a", true, 90_000, 1);
    sink.project(&same).await.unwrap();
    sink.project(&same).await.unwrap();
    let input = sink.selection_input("code-agent");
    assert_eq!(input.len(), 1);
    assert_eq!(input[0].receipt_count, 1);
}
