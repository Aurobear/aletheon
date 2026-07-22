use executive::r#impl::daemon::bootstrap::production_embodiment::ProductionStartupGate;

#[test]
fn full_gate_sequence_passes() {
    let mut gate = ProductionStartupGate::new();
    gate.check_config(Ok(())).unwrap();
    gate.check_credentials(true).unwrap();
    gate.check_device_identity(true, true).unwrap();
    gate.check_manifest_digest(true, true).unwrap();
    gate.check_evidence(true, true).unwrap();
    gate.check_estop(true).unwrap();
    gate.check_operator_arming(Some("operator-1")).unwrap();
    assert!(gate.finalize().is_ok());
}

#[test]
fn missing_operator_arming_fails() {
    let mut gate = ProductionStartupGate::new();
    gate.check_config(Ok(())).unwrap();
    gate.check_credentials(true).unwrap();
    gate.check_device_identity(true, true).unwrap();
    gate.check_manifest_digest(true, true).unwrap();
    gate.check_evidence(true, true).unwrap();
    gate.check_estop(true).unwrap();
    assert!(gate.check_operator_arming(None).is_err());
}

#[test]
fn never_downgrade_to_simulator() {
    // ProductionStartupGate::finalize() returns Err on failure.
    // It NEVER returns Ok(("simulator")) to silently downgrade.
    let mut gate = ProductionStartupGate::new();
    gate.check_config(Err(vec!["bad config".into()])).ok();
    assert!(gate.finalize().is_err());
}
