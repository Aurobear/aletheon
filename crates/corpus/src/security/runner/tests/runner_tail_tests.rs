use super::*;

#[tokio::test]
async fn runner_execpolicy_prompt_triggers_approval() {
    // Build an execpolicy that prompts for "bash_exec".
    let mut policy = ExecPolicy::new();
    policy.add_rule(ExecPrefixRule::new("bash_exec", ExecDecision::Prompt));

    let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
    let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger, test_clock())
        .with_approval_gate(Arc::new(AutoApproveGate))
        .with_policy(policy);

    let tool = DummyL2Tool;
    let result = runner
        .execute_tool(
            &tool,
            serde_json::json!({
                "command": "rm -rf /tmp/test",
                "network_enabled": true
            }),
            &make_ctx(),
            "t1",
        )
        .await;
    assert!(
        matches!(result, Ok(_) | Err(ToolError::OutputRejected(_))),
        "approval should pass before output validation: {result:?}"
    );
}

#[tokio::test]
async fn runner_no_execpolicy_falls_back_to_policy_engine() {
    // Without with_policy(), the inline PolicyEngine is used.
    let audit_logger = AuditLogger::new(std::path::PathBuf::from("/dev/null")).unwrap();
    let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger, test_clock())
        .with_approval_gate(Arc::new(AutoApproveGate));

    let tool = DummyL2Tool;
    let result = runner
        .execute_tool(
            &tool,
            serde_json::json!({
                "command": "rm -rf /tmp/test",
                "network_enabled": true
            }),
            &make_ctx(),
            "t1",
        )
        .await;
    assert!(
        matches!(result, Ok(_) | Err(ToolError::OutputRejected(_))),
        "inline policy should pass before output validation: {result:?}"
    );
}

#[test]
fn build_command_vec_extracts_bash_command() {
    let input = serde_json::json!({ "command": "rm -rf /tmp/test" });
    let cmd = ToolRunnerWithGuard::build_command_vec("bash_exec", &input);
    // Command string is kept as a single token to preserve shell syntax.
    assert_eq!(cmd, vec!["bash_exec", "rm -rf /tmp/test"]);
}

#[test]
fn build_command_vec_non_bash_tool() {
    let input = serde_json::json!({ "path": "/tmp/file.txt" });
    let cmd = ToolRunnerWithGuard::build_command_vec("file_read", &input);
    assert_eq!(cmd, vec!["file_read"]);
}

#[tokio::test]
async fn managed_command_policy_uses_host_classified_effects() {
    let runner = make_runner(Arc::new(AutoApproveGate));
    assert!(matches!(
        runner.check_policy(
            "exec_command",
            &serde_json::json!({"command":"which glab && glab --version"})
        ),
        PolicyVerdict::Allow
    ));
    assert!(matches!(
        runner.check_policy(
            "exec_command",
            &serde_json::json!({"command":"glab mr view 22 --repo owner/repo"})
        ),
        PolicyVerdict::RequireApproval { .. }
    ));
    assert!(matches!(
        runner.check_policy(
            "exec_command",
            &serde_json::json!({"command":"sudo apt-get install glab"})
        ),
        PolicyVerdict::Deny { .. }
    ));
}

#[tokio::test]
async fn audit_record_preserves_session_and_dynamic_command_risk() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("audit.jsonl");
    let runner = ToolRunnerWithGuard::with_default_sandbox(
        AuditLogger::new(path.clone()).unwrap(),
        test_clock(),
    );
    runner
        .log_audit(
            fabric::AuditEventId::new(),
            "exec_command",
            &serde_json::json!({"command":"which glab"}),
            PermissionLevel::L1,
            "turn-1",
            "session-1",
            None,
            &fabric::MonoTime(0),
            "Allow",
        )
        .await
        .unwrap();

    let record: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(path).unwrap().trim()).unwrap();
    assert_eq!(record["session_id"], "session-1");
    assert_eq!(record["risk_category"], "ReadOnly");
}
