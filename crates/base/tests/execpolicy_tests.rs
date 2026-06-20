use base::execpolicy::*;

fn strs(cmd: &[&str]) -> Vec<String> {
    cmd.iter().map(|s| s.to_string()).collect()
}

#[test]
fn decision_ordering() {
    assert!(Decision::Allow < Decision::Prompt);
    assert!(Decision::Prompt < Decision::Forbidden);
}

#[test]
fn prefix_rule_matches_command() {
    // Pattern matches: first arg must be one of -rf or -r
    let rule = PrefixRule::new("rm", Decision::Forbidden)
        .with_pattern(vec![
            PatternToken::Alternatives(vec!["-rf".into(), "-r".into()]),
        ]);

    assert!(rule.matches(&strs(&["rm", "-rf", "/"])).is_some());
    assert!(rule.matches(&strs(&["rm", "-r", "dir"])).is_some());
    // "rm file" has no matching first arg
    assert!(rule.matches(&strs(&["rm", "file"])).is_none());
}

#[test]
fn prefix_rule_no_pattern_matches_any_invocation() {
    let rule = PrefixRule::new("ls", Decision::Allow);
    assert!(rule.matches(&strs(&["ls", "-la"])).is_some());
    assert!(rule.matches(&strs(&["ls"])).is_some());
}

#[test]
fn prefix_rule_exact_token() {
    let rule = PrefixRule::new("dd", Decision::Forbidden)
        .with_pattern(vec![PatternToken::Exact("if=".into())]);
    assert!(rule.matches(&strs(&["dd", "if=/dev/zero"])).is_none()); // "if=/dev/zero" != "if="
    assert!(rule.matches(&strs(&["dd", "of=file"])).is_none());
}

#[test]
fn policy_check_allows_safe_commands() {
    let policy = Policy::new();
    let eval = policy.check(&strs(&["ls", "-la"]), default_heuristics);
    assert_eq!(eval.decision, Decision::Allow);
}

#[test]
fn policy_check_forbids_dangerous_commands() {
    let policy = Policy::new();
    let eval = policy.check(&strs(&["rm", "-rf", "/"]), default_heuristics);
    assert_eq!(eval.decision, Decision::Forbidden);
}

#[test]
fn policy_check_prompts_unknown_commands() {
    let policy = Policy::new();
    let eval = policy.check(&strs(&["some_unknown_tool"]), default_heuristics);
    assert_eq!(eval.decision, Decision::Prompt);
}

#[test]
fn policy_overlay_merge_precedence() {
    let mut base = Policy::new();
    // Base allows rm (unrealistic but tests merge)
    base.add_rule(PrefixRule::new("rm", Decision::Allow));

    let mut overlay = Policy::new();
    // Overlay forbids rm
    overlay.add_rule(PrefixRule::new("rm", Decision::Forbidden));

    base.merge_overlay(overlay);

    // Overlay takes precedence (last-wins)
    let eval = base.check(&strs(&["rm", "-rf", "/"]), default_heuristics);
    assert_eq!(eval.decision, Decision::Forbidden);
}

#[test]
fn policy_layered_load() {
    let system = r#"
[[rules]]
program = "rm"
decision = "prompt"
"#;

    let user = r#"
[[rules]]
program = "rm"
decision = "forbidden"
"#;

    let policy = load_policy_layered(Some(system), Some(user), None).unwrap();
    let eval = policy.check(&strs(&["rm", "-rf", "/"]), default_heuristics);
    // User overrides system (last-wins via merge_overlay)
    assert_eq!(eval.decision, Decision::Forbidden);
}

#[test]
fn network_rule_check() {
    let mut policy = Policy::new();
    policy.add_network_rule(NetworkRule {
        host: "evil.com".into(),
        protocol: NetworkProtocol::Https,
        decision: Decision::Forbidden,
    });

    let eval = policy.check_network("evil.com", NetworkProtocol::Https);
    assert_eq!(eval.decision, Decision::Forbidden);

    let eval = policy.check_network("safe.com", NetworkProtocol::Https);
    assert_eq!(eval.decision, Decision::Allow);
}

#[test]
fn empty_command_returns_prompt() {
    let policy = Policy::new();
    let eval = policy.check(&[], default_heuristics);
    assert_eq!(eval.decision, Decision::Prompt);
}

#[test]
fn default_heuristics_safe_commands() {
    assert_eq!(default_heuristics(&strs(&["cat", "file"])), Decision::Allow);
    assert_eq!(default_heuristics(&strs(&["pwd"])), Decision::Allow);
    assert_eq!(default_heuristics(&strs(&["echo", "hello"])), Decision::Allow);
}

#[test]
fn default_heuristics_dangerous_commands() {
    assert_eq!(default_heuristics(&strs(&["rm", "-rf", "/"])), Decision::Forbidden);
    assert_eq!(default_heuristics(&strs(&["mkfs", "/dev/sda"])), Decision::Forbidden);
    assert_eq!(default_heuristics(&strs(&["shutdown", "now"])), Decision::Forbidden);
}

#[test]
fn load_policy_from_str_valid() {
    let toml = r#"
[[rules]]
program = "git"
decision = "allow"
"#;
    let policy = load_policy_from_str(toml).unwrap();
    let eval = policy.check(&strs(&["git", "status"]), default_heuristics);
    assert_eq!(eval.decision, Decision::Allow);
}

#[test]
fn load_policy_from_str_invalid() {
    let result = load_policy_from_str("not valid toml [[[");
    assert!(result.is_err());
}
