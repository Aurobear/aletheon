use super::super::approval_gate::bootstrap_workspace_trust_resolver;

use super::*;
use fabric::dasein::{OutcomeStatus, SelfVersion};

#[tokio::test]
async fn daemon_bootstrap_keeps_untrusted_repo_files_usable_without_loading_executable_config() {
    use fabric::workspace_trust::{ClientMode, ExecutableConfigSource, WorkspaceTrustDecision};

    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("untrusted-repository");
    let data_dir = root.path().join("daemon-state");
    std::fs::create_dir_all(repository.join(".git")).unwrap();
    std::fs::write(
        repository.join(".git/config"),
        "[remote \"origin\"]\nurl = https://example.test/untrusted.git\n",
    )
    .unwrap();
    std::fs::create_dir_all(repository.join(".grok/hooks")).unwrap();
    std::fs::create_dir_all(repository.join("agents")).unwrap();
    let sentinel = repository.join("executable-config-ran");
    let hook = repository.join(".grok/hooks/pre-turn");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\nprintf ran > '{}'\n", sentinel.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(
        repository.join(".grok/mcp.json"),
        format!(
            r#"{{"command":"/bin/sh","args":["-c","touch {}"]}}"#,
            sentinel.display()
        ),
    )
    .unwrap();
    std::fs::write(
        repository.join("agents/sentinel.toml"),
        format!("command = \"touch {}\"\n", sentinel.display()),
    )
    .unwrap();
    std::fs::write(
        repository.join(".envrc"),
        format!("touch '{}'\n", sentinel.display()),
    )
    .unwrap();

    let canonical = std::fs::canonicalize(&repository).unwrap();
    assert_ne!(canonical, std::path::Path::new("/"));
    let workspace = fabric::WorkspacePolicy::from_resolved_roots(canonical.clone(), vec![])
        .expect("a non-root repository is a valid daemon workspace");
    let resolver = bootstrap_workspace_trust_resolver(&data_dir, true, None);
    let decision = resolver
        .evaluate(
            fabric::PrincipalId("headless-daemon".into()),
            crate::service::workspace_trust::workspace_identity(workspace.cwd()),
            ClientMode::Headless,
            crate::service::workspace_trust::is_broad_unrecordable_root(workspace.cwd()),
            1,
        )
        .await;
    let WorkspaceTrustDecision::Restricted { blocked } = &decision else {
        panic!("headless bootstrap must restrict an unrecorded repository: {decision:?}");
    };
    for source in [
        ExecutableConfigSource::RepoHooks,
        ExecutableConfigSource::RepoMcpServer,
        ExecutableConfigSource::EnvrcLoader,
        ExecutableConfigSource::RepoAgentCommand,
    ] {
        assert!(
            blocked.contains(&source),
            "missing blocked source {source:?}"
        );
    }

    // Production loaders consume only granted categories. This stub is a
    // sentinel for every executable loader boundary and must remain idle.
    let mut loader_calls = Vec::new();
    for source in ExecutableConfigSource::all() {
        if crate::service::workspace_trust::source_is_granted(&decision, source) {
            loader_calls.push(source);
            std::fs::write(&sentinel, "ran").unwrap();
        }
    }
    assert!(loader_calls.is_empty());
    assert!(!sentinel.exists());

    // Folder trust gates executable configuration, not ordinary workspace
    // authority: normal reads and writes remain available after bootstrap.
    let ordinary = workspace.cwd().join("ordinary.txt");
    std::fs::write(&ordinary, "ordinary workspace data").unwrap();
    assert_eq!(
        std::fs::read_to_string(ordinary).unwrap(),
        "ordinary workspace data"
    );
    assert!(!sentinel.exists());
}

#[tokio::test]
async fn daemon_self_field_bootstrap_replays_before_new_transitions() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("self_field.db");

    let mut first = SelfField::new(SelfFieldConfig {
        db_path: Some(database.clone()),
        clock: Some(Arc::new(kernel::chronos::TestClock::new(100, 0))),
        ..Default::default()
    });
    initialize_self_field(&mut first, root.path())
        .await
        .unwrap();
    first
        .dasein()
        .unwrap()
        .record_outcome("first boot", OutcomeStatus::Succeeded, "daemon-bootstrap")
        .await
        .unwrap();
    first.shutdown().await.unwrap();

    let mut restarted = SelfField::new(SelfFieldConfig {
        db_path: Some(database),
        clock: Some(Arc::new(kernel::chronos::TestClock::new(5_100, 0))),
        ..Default::default()
    });
    initialize_self_field(&mut restarted, root.path())
        .await
        .unwrap();
    assert!(matches!(
        restarted.health().await,
        fabric::SubsystemHealth::Healthy
    ));
    let dasein = restarted.dasein_handle().unwrap();
    assert_eq!(dasein.self_version().await, SelfVersion(2));
    let receipt = dasein
        .record_outcome(
            "first turn after restart",
            OutcomeStatus::Succeeded,
            "daemon-bootstrap",
        )
        .await
        .unwrap();
    assert_eq!(receipt.previous_version, SelfVersion(2));
    assert_eq!(receipt.current_version, SelfVersion(3));
    restarted.shutdown().await.unwrap();
}
