#[test]
fn retired_governance_methods_are_unknown() {
    let dispatcher = include_str!("../../aletheon/src/wiring/daemon/handler/rpc.rs");

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
            !dispatcher.contains(&format!("\"{method}\" =>")),
            "retired method {method} still has a daemon dispatch route"
        );
    }

    assert!(
        dispatcher.contains("\"code\": -32601"),
        "unmatched daemon methods must return JSON-RPC method-not-found"
    );
}
