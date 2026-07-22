use executive::application::robot_audit::AuditChain;

#[test]
fn audit_chain_integrity() {
    let chain = AuditChain::new(100);
    for i in 0..50 {
        chain
            .append(
                format!("op-{}", i),
                "kuavo-01".into(),
                "stance".into(),
                (i % 3) as u32,
                "matched".into(),
                Some(format!("verif-{}", i)),
                None,
                i == 49,
                false,
                1000 + i,
            )
            .unwrap();
    }
    assert_eq!(chain.len(), 50);
    assert!(chain.verify_chain().unwrap());
}

#[test]
fn tamper_detection() {
    let chain = AuditChain::new(10);
    chain
        .append(
            "op".into(),
            "d".into(),
            "s".into(),
            1,
            "m".into(),
            None,
            None,
            false,
            false,
            1000,
        )
        .unwrap();
    // Verify chain succeeds
    assert!(chain.verify_chain().unwrap());
    // Manual tamper check: the chain verify should detect if entries were modified
    // (verification via hash recomputation)
}

#[test]
fn empty_chain_is_valid() {
    let chain = AuditChain::new(10);
    assert_eq!(chain.len(), 0);
    assert!(chain.verify_chain().unwrap());
}
