#[test]
fn client_protocol_does_not_publish_manual_governance_methods() {
    let source = include_str!("../src/protocol/client.rs");
    for method in [
        "reflect",
        "reflect_now",
        "evolution",
        "genome",
        "hooks_list",
        "plan_approve",
        "host.computer",
    ] {
        assert!(
            !source.contains(&format!("\"{method}\"")),
            "published retired method {method}"
        );
    }
}
