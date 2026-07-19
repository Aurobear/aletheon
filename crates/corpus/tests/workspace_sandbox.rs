use std::sync::Arc;
use std::time::Duration;

use corpus::security::sandbox::{
    BubblewrapBackend, BwrapBuilder, FilesystemPolicy, SandboxBackend, SandboxConfig,
};
use fabric::WorkspacePolicy;
use kernel::chronos::TestClock;

fn count_triplet(args: &[String], flag: &str, source: &str, target: &str) -> usize {
    args.windows(3)
        .filter(|items| *items == [flag, source, target])
        .count()
}

fn position(args: &[String], flag: &str, path: &str) -> usize {
    args.windows(3)
        .position(|items| items == [flag, path, path])
        .unwrap_or_else(|| panic!("missing {flag} {path} {path} in {args:?}"))
}

#[test]
fn protected_mounts_follow_every_writable_bind_and_cwd_is_not_rebound() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let shared = temp.path().join("shared");
    for root in [&project, &shared] {
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join(".aletheon")).unwrap();
    }
    let workspace =
        WorkspacePolicy::from_resolved_roots(project.clone(), vec![shared.clone()]).unwrap();
    let policy = FilesystemPolicy::from_workspace(&workspace);
    let config = SandboxConfig {
        workspace,
        environment: Default::default(),
        policy: None,
    };
    let args = BwrapBuilder::new(policy).build_args("true", &config);

    for root in [&project, &shared] {
        let root = root.to_string_lossy();
        let writable = position(&args, "--bind", &root);
        for protected_name in [".git", ".aletheon"] {
            let protected = root.to_string() + "/" + protected_name;
            assert!(position(&args, "--ro-bind", &protected) > writable);
        }
        assert_eq!(count_triplet(&args, "--bind", &root, &root), 1);
    }
}

#[tokio::test]
async fn bubblewrap_enforces_workspace_and_protected_paths_when_available() {
    let Some(backend) = BubblewrapBackend::probe(Arc::new(TestClock::default())) else {
        println!("SKIP: bwrap unavailable");
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let shared = temp.path().join("shared");
    let outside = temp.path().join("outside");
    for root in [&project, &shared, &outside] {
        std::fs::create_dir_all(root).unwrap();
    }
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(shared.join(".aletheon")).unwrap();
    let config = SandboxConfig {
        workspace: WorkspacePolicy::from_resolved_roots(project.clone(), vec![shared.clone()])
            .unwrap(),
        environment: Default::default(),
        policy: None,
    };

    let probe = backend
        .execute("true", &config, Duration::from_secs(5))
        .await;
    if probe.as_ref().is_err() || probe.as_ref().is_ok_and(|result| result.exit_code != 0) {
        println!("SKIP: bwrap user namespace unavailable");
        return;
    }

    let allowed = format!(
        "touch '{}' '{}'",
        project.join("cwd-ok").display(),
        shared.join("add-ok").display()
    );
    let result = backend
        .execute(&allowed, &config, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    assert!(project.join("cwd-ok").exists());
    assert!(shared.join("add-ok").exists());

    for denied in [
        outside.join("denied"),
        project.join(".git/denied"),
        shared.join(".aletheon/denied"),
    ] {
        let result = backend
            .execute(
                &format!("touch '{}'", denied.display()),
                &config,
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_ne!(
            result.exit_code,
            0,
            "unexpected write to {}",
            denied.display()
        );
        assert!(!denied.exists());
    }
}
