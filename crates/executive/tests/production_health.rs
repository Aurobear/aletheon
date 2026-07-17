use executive::r#impl::health::{ComponentHealth, HealthClass, HealthRegistry};

#[test]
fn required_store_failure_is_unready_but_optional_outages_are_degraded() {
    let health = HealthRegistry::production_ready();
    health.set("gbrain_spool", ComponentHealth::degraded("schema_drift"));
    health.set("google_sync", ComponentHealth::degraded("auth_unavailable"));
    let snapshot = health.snapshot();
    assert_eq!(snapshot.liveness, "alive");
    assert_eq!(snapshot.readiness, "degraded");

    health.set(
        "objective_store",
        ComponentHealth::unready("database_corrupt"),
    );
    let snapshot = health.snapshot();
    assert_eq!(snapshot.readiness, "unready");
    assert_eq!(
        snapshot.components["objective_store"].class,
        HealthClass::RequiredUnready
    );
}

#[test]
fn full_disk_stale_sync_and_overdue_backup_use_counts_and_categories_only() {
    let health = HealthRegistry::production_ready();
    let mut disk = ComponentHealth::unready("minimum_free_space");
    disk.count = Some(0);
    health.set("disk_quota", disk);
    let mut sync = ComponentHealth::degraded("sync_stale");
    sync.age_seconds = Some(7_200);
    health.set("google_sync", sync);
    let mut backup = ComponentHealth::degraded("backup_overdue");
    backup.age_seconds = Some(200_000);
    health.set("backup", backup);

    let encoded = serde_json::to_string(&health.snapshot()).unwrap();
    assert!(encoded.contains("minimum_free_space"));
    assert!(encoded.contains("sync_stale"));
    assert!(encoded.contains("backup_overdue"));
    for forbidden in [
        "Bearer",
        "token",
        "password",
        "@example.com",
        "/etc/aletheon/credentials",
    ] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn startup_recovery_and_shutdown_are_explicit() {
    let health = HealthRegistry::production_ready();
    assert_eq!(health.snapshot().readiness, "ready");
    health.set("approvals_outbox", ComponentHealth::ready());
    health.set(
        "worktree_capacity",
        ComponentHealth::degraded("capacity_low"),
    );
    assert_eq!(health.snapshot().readiness, "degraded");
    health.begin_shutdown();
    let snapshot = health.snapshot();
    assert_eq!(snapshot.readiness, "unready");
    assert_eq!(
        snapshot.components["daemon"].error_category,
        Some("shutting_down")
    );
}

#[test]
fn agent_recovery_health_exposes_only_sanitized_counts_and_blocks_readiness() {
    let health = HealthRegistry::production_ready();
    let mut recovering = ComponentHealth::unready("agent_recovery_incomplete");
    recovering.count = Some(2);
    health.set("agent_recovery", recovering);
    let encoded = serde_json::to_string(&health.snapshot()).unwrap();
    assert!(encoded.contains("agent_recovery_incomplete"));
    assert!(encoded.contains("\"count\":2"));
    assert_eq!(health.snapshot().readiness, "unready");
    for forbidden in ["checkpoint:", "process:", "agent:", "/worktree/"] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn enabled_goal_worker_failure_is_required_unready() {
    let health = HealthRegistry::production_ready();
    health.set("goal_worker", ComponentHealth::unready("worker_stopped"));

    let snapshot = health.snapshot();
    assert_eq!(snapshot.readiness, "unready");
    assert_eq!(
        snapshot.components["goal_worker"].error_category,
        Some("worker_stopped")
    );
}

#[test]
fn health_contract_never_accepts_arbitrary_error_bodies() {
    // Component health intentionally has no message/address/path/provider-body field.
    let fields = serde_json::to_value(ComponentHealth::degraded("worker_stopped")).unwrap();
    assert_eq!(fields.as_object().unwrap().len(), 2);
    assert!(fields.get("error_category").is_some());
}

#[test]
fn unix_server_retains_peer_credential_gate() {
    let server = include_str!("../src/impl/daemon/server.rs");
    assert!(server.contains("check_peer_cred"));
    assert!(server.contains("Connection rejected by peer credential check"));
    let script = include_str!("../../../scripts/aletheon-healthcheck.sh");
    assert!(script.contains("AF_UNIX"));
    assert!(script.contains("\"ready\": 0, \"degraded\": 1, \"unready\": 2"));
}

#[test]
fn live_storage_probe_reports_free_bytes_and_backup_marker_age() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("state")).unwrap();
    let health = HealthRegistry::production_ready();
    health.refresh_storage(directory.path(), 1, true, 60);
    let snapshot = health.snapshot();
    assert_eq!(snapshot.components["disk_quota"].class, HealthClass::Ready);
    assert!(snapshot.components["disk_quota"].count.unwrap() > 0);
    assert_eq!(
        snapshot.components["backup"].error_category,
        Some("backup_missing")
    );

    std::fs::write(directory.path().join("state/backup-marker.json"), "{}").unwrap();
    health.refresh_storage(directory.path(), 1, true, 60);
    let snapshot = health.snapshot();
    assert_eq!(snapshot.components["backup"].class, HealthClass::Ready);
    assert!(snapshot.components["backup"].age_seconds.is_some());
}
