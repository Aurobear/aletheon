//! Shared native-backend contract suite. Each native target supplies only factories;
//! behavioral assertions remain identical.

use platform::error::HostErrorKind;
use platform::filesystem::{
    AtomicWrite, FilesystemAccess, FilesystemHost, FilesystemScope, SymlinkPolicy,
};
use platform::path::HostPath;
use platform::process::{ProcessHost, ProcessSignal, SpawnSpec};
use platform::pty::{PtyHost, PtySize};
use platform::sandbox::{SandboxHost, SandboxProfile, SandboxStrength};
use platform::service::ServiceHost;

fn any_filesystem_host(root: &std::path::Path) -> Box<dyn FilesystemHost> {
    Box::new(
        platform::backend::linux::LinuxFilesystemHost::scoped(FilesystemScope {
            roots: vec![HostPath::new(root.to_path_buf())],
            readable_paths: vec![],
            access: FilesystemAccess::ReadWrite,
            symlink_policy: SymlinkPolicy::WithinRoot,
        })
        .unwrap(),
    )
}

fn any_process_host() -> Box<dyn ProcessHost> {
    Box::new(platform::backend::linux::LinuxProcessHost::new())
}

fn any_pty_host() -> Box<dyn PtyHost> {
    Box::new(platform::backend::linux::LinuxPtyHost::new())
}

fn any_sandbox_host() -> Box<dyn SandboxHost> {
    Box::new(platform::backend::linux::LinuxSandboxHost::new())
}

fn any_service_host() -> Box<dyn ServiceHost> {
    Box::new(platform::backend::linux::LinuxServiceHost::new())
}

#[tokio::test]
async fn filesystem_write_read_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let host = any_filesystem_host(tmp.path());
    let p = HostPath::new(tmp.path().join("contract_test.txt"));

    let receipt = host
        .atomic_write(AtomicWrite {
            path: p.clone(),
            content: b"contract".to_vec(),
            expected_sha256: None,
            mode: None,
        })
        .await
        .unwrap();
    assert_eq!(receipt.bytes_written, 8);

    let data = host.read(&p).await.unwrap();
    assert_eq!(data, b"contract");
}

#[tokio::test]
async fn filesystem_remove_is_scoped_and_durable() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = HostPath::new(root.path().join("remove.txt"));
    let external = HostPath::new(outside.path().join("keep.txt"));
    std::fs::write(target.native(), b"remove").unwrap();
    std::fs::write(external.native(), b"keep").unwrap();
    let host = any_filesystem_host(root.path());

    let receipt = host
        .remove_file(platform::RemoveFile {
            path: target.clone(),
            expected_sha256: None,
        })
        .await
        .unwrap();
    assert!(receipt.success);
    assert!(!target.native().exists());

    let error = host
        .remove_file(platform::RemoveFile {
            path: external.clone(),
            expected_sha256: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(error.kind, HostErrorKind::PermissionDenied(_)));
    assert!(external.native().exists());
}

#[tokio::test]
async fn filesystem_remove_rejects_a_stale_precondition() {
    let root = tempfile::tempdir().unwrap();
    let target = HostPath::new(root.path().join("keep.txt"));
    std::fs::write(target.native(), b"current").unwrap();
    let host = any_filesystem_host(root.path());

    let error = host
        .remove_file(platform::RemoveFile {
            path: target.clone(),
            expected_sha256: Some("0".repeat(64)),
        })
        .await
        .unwrap_err();

    assert!(matches!(error.kind, HostErrorKind::Conflict(_)));
    assert!(target.native().exists());
}

#[tokio::test]
async fn filesystem_creates_nested_directories_inside_scope() {
    let root = tempfile::tempdir().unwrap();
    let host = any_filesystem_host(root.path());
    let nested = HostPath::new(root.path().join("one/two"));
    let receipt = host.create_dir_all(&nested).await.unwrap();
    assert!(receipt.success);
    assert!(nested.native().is_dir());
}

#[cfg(unix)]
#[tokio::test]
async fn filesystem_directory_creation_rejects_symlink_escape() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    let host = any_filesystem_host(root.path());
    let error = host
        .create_dir_all(&HostPath::new(root.path().join("escape/new")))
        .await
        .unwrap_err();
    assert!(matches!(error.kind, HostErrorKind::PermissionDenied(_)));
    assert!(!outside.path().join("new").exists());
}

#[tokio::test]
async fn process_spawn_and_terminate() {
    let host = any_process_host();
    let (pid, receipt) = host
        .spawn(SpawnSpec {
            argv: vec!["true".into()],
            env: vec![],
            working_dir: None,
            timeout_ms: None,
        })
        .await
        .unwrap();
    assert!(receipt.success);
    // Verify signal contract works
    let result = host.signal(pid, ProcessSignal::Kill).await;
    assert!(result.is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn process_tree_termination_reaps_root_and_descendant() {
    let root = tempfile::tempdir().unwrap();
    let child_pid = root.path().join("child.pid");
    let host = any_process_host();
    let (pid, _) = host
        .spawn(SpawnSpec {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "sleep 30 & echo $! > child.pid; wait".into(),
            ],
            env: vec![],
            working_dir: Some(HostPath::new(root.path().to_path_buf())),
            timeout_ms: None,
        })
        .await
        .unwrap();
    for _ in 0..50 {
        if child_pid.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let descendant: i32 = std::fs::read_to_string(&child_pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let receipt = host.terminate_tree(pid, 10).await.unwrap();
    assert!(receipt.success);
    for _ in 0..50 {
        if unsafe { libc::kill(descendant, 0) } != 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("descendant remained alive after process-tree termination");
}

#[tokio::test]
async fn stale_write_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let host = any_filesystem_host(tmp.path());
    let p = HostPath::new(tmp.path().join("stale.txt"));
    std::fs::write(p.native(), b"original").unwrap();

    let result = host
        .atomic_write(AtomicWrite {
            path: p,
            content: b"new".to_vec(),
            expected_sha256: Some(
                "0000000000000000000000000000000000000000000000000000000000000000".into(),
            ),
            mode: None,
        })
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn filesystem_rejects_paths_outside_operation_scope() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let host = any_filesystem_host(root.path());
    let error = host
        .read(&HostPath::new(outside.path().join("secret")))
        .await
        .expect_err("an operation-scoped host must reject another root");
    assert!(matches!(error.kind, HostErrorKind::PermissionDenied(_)));
}

#[cfg(unix)]
#[tokio::test]
async fn filesystem_rejects_symlink_escape() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"secret").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    let host = any_filesystem_host(root.path());

    let error = host
        .read(&HostPath::new(root.path().join("escape/secret")))
        .await
        .expect_err("a symlink must not escape the operation scope");
    assert!(matches!(error.kind, HostErrorKind::PermissionDenied(_)));
}

#[cfg(unix)]
#[tokio::test]
async fn filesystem_root_replacement_never_redirects_scoped_operations() {
    let parent = tempfile::tempdir().unwrap();
    let admitted = parent.path().join("admitted");
    let moved = parent.path().join("moved-admitted");
    let outside = parent.path().join("outside");
    std::fs::create_dir(&admitted).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(admitted.join("victim.txt"), b"inside").unwrap();
    std::fs::write(outside.join("victim.txt"), b"outside-sentinel").unwrap();

    let host = any_filesystem_host(&admitted);
    std::fs::rename(&admitted, &moved).unwrap();
    std::os::unix::fs::symlink(&outside, &admitted).unwrap();

    let write_result = host
        .atomic_write(AtomicWrite {
            path: HostPath::new(admitted.join("created.txt")),
            content: b"scoped".to_vec(),
            expected_sha256: None,
            mode: None,
        })
        .await;
    if write_result.is_ok() {
        assert_eq!(std::fs::read(moved.join("created.txt")).unwrap(), b"scoped");
    }

    let read_result = host.read(&HostPath::new(admitted.join("victim.txt"))).await;
    if let Ok(content) = read_result {
        assert_eq!(content, b"inside");
    }

    let remove_result = host
        .remove_file(platform::RemoveFile {
            path: HostPath::new(admitted.join("victim.txt")),
            expected_sha256: None,
        })
        .await;
    if remove_result.is_ok() {
        assert!(!moved.join("victim.txt").exists());
    }

    assert_eq!(
        std::fs::read(outside.join("victim.txt")).unwrap(),
        b"outside-sentinel"
    );
    assert!(!outside.join("created.txt").exists());
    assert!(std::fs::read_dir(&outside).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[cfg(unix)]
#[tokio::test]
async fn filesystem_parent_swap_pressure_never_escapes_atomic_write() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let active_parent = root.path().join("workspace");
    let parked_parent = root.path().join("workspace-admitted");
    let outside_sentinel = outside.path().join("sentinel.txt");
    std::fs::create_dir(&active_parent).unwrap();
    std::fs::write(&outside_sentinel, b"outside-sentinel").unwrap();
    let host = any_filesystem_host(root.path());

    let stop = Arc::new(AtomicBool::new(false));
    let attacker_stop = Arc::clone(&stop);
    let attacker_active = active_parent.clone();
    let attacker_parked = parked_parent.clone();
    let attacker_outside = outside.path().to_path_buf();
    let attacker = std::thread::spawn(move || {
        while !attacker_stop.load(Ordering::Acquire) {
            std::fs::rename(&attacker_active, &attacker_parked).unwrap();
            std::os::unix::fs::symlink(&attacker_outside, &attacker_active).unwrap();
            std::thread::yield_now();
            std::fs::remove_file(&attacker_active).unwrap();
            std::fs::rename(&attacker_parked, &attacker_active).unwrap();
        }
    });

    for sequence in 0..100 {
        let _ = host
            .atomic_write(AtomicWrite {
                path: HostPath::new(active_parent.join("result.txt")),
                content: format!("write-{sequence}").into_bytes(),
                expected_sha256: None,
                mode: None,
            })
            .await;
    }
    stop.store(true, Ordering::Release);
    attacker.join().unwrap();

    assert_eq!(
        std::fs::read(&outside_sentinel).unwrap(),
        b"outside-sentinel"
    );
    assert!(!outside.path().join("result.txt").exists());
    assert!(std::fs::read_dir(outside.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[tokio::test]
async fn readonly_filesystem_scope_rejects_writes() {
    let root = tempfile::tempdir().unwrap();
    let host = platform::backend::linux::LinuxFilesystemHost::scoped(FilesystemScope {
        roots: vec![HostPath::new(root.path().to_path_buf())],
        readable_paths: vec![],
        access: FilesystemAccess::ReadOnly,
        symlink_policy: SymlinkPolicy::WithinRoot,
    })
    .unwrap();
    let error = host
        .atomic_write(AtomicWrite {
            path: HostPath::new(root.path().join("denied")),
            content: b"no".to_vec(),
            expected_sha256: None,
            mode: None,
        })
        .await
        .expect_err("read-only scopes must reject writes");
    assert!(matches!(error.kind, HostErrorKind::PermissionDenied(_)));
}

#[tokio::test]
async fn explicitly_admitted_external_file_is_readable_but_its_sibling_is_not() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let admitted = outside.path().join("admitted.txt");
    let sibling = outside.path().join("sibling.txt");
    std::fs::write(&admitted, b"allowed").unwrap();
    std::fs::write(&sibling, b"denied").unwrap();
    let host = platform::backend::linux::LinuxFilesystemHost::scoped(FilesystemScope {
        roots: vec![HostPath::new(root.path().to_path_buf())],
        readable_paths: vec![HostPath::new(admitted.clone())],
        access: FilesystemAccess::ReadOnly,
        symlink_policy: SymlinkPolicy::WithinRoot,
    })
    .unwrap();

    assert_eq!(
        host.read(&HostPath::new(admitted)).await.unwrap(),
        b"allowed"
    );
    let error = host.read(&HostPath::new(sibling)).await.unwrap_err();
    assert!(matches!(error.kind, HostErrorKind::PermissionDenied(_)));
}

#[tokio::test]
async fn process_timeout_is_a_typed_error() {
    let host = any_process_host();
    let error = host
        .spawn(SpawnSpec {
            argv: vec!["sleep".into(), "10".into()],
            env: vec![],
            working_dir: None,
            timeout_ms: Some(10),
        })
        .await
        .expect_err("a timed out process must not return an Ok receipt");

    assert!(matches!(error.kind, HostErrorKind::Timeout(_)));
}

#[tokio::test]
async fn signal_rejects_process_group_zero() {
    let host = any_process_host();
    let error = host
        .signal(platform::ProcessId(0), ProcessSignal::Kill)
        .await
        .expect_err("PID 0 targets the caller's process group and must be rejected");

    assert!(matches!(error.kind, HostErrorKind::NotFound(_)));
}

#[tokio::test]
async fn pty_supports_resize_and_close_receipts() {
    let mut channel = any_pty_host()
        .open(PtySize { rows: 24, cols: 80 })
        .await
        .expect("the native Linux backend must open /dev/ptmx");

    let resize = channel
        .resize(PtySize {
            rows: 40,
            cols: 120,
        })
        .await
        .expect("native PTY resize must be implemented");
    assert!(resize.success);
    assert_eq!(resize.operation, "pty_resize");

    let close = channel.close().await.expect("PTY close must be bounded");
    assert!(close.success);
    assert_eq!(close.operation, "pty_close");
}

#[tokio::test]
async fn sandbox_probe_only_reports_observed_strengths() {
    let strengths = any_sandbox_host().probe().await;
    assert!(!strengths.is_empty());
    assert_eq!(
        strengths.contains(&SandboxStrength::Namespace),
        std::path::Path::new("/proc/self/ns/user").exists()
    );
    assert_eq!(
        strengths.contains(&SandboxStrength::Seccomp),
        std::path::Path::new("/proc/sys/kernel/seccomp/actions_avail").exists()
    );
}

#[tokio::test]
async fn unavailable_sandbox_apply_fails_closed() {
    let error = any_sandbox_host()
        .apply(&SandboxProfile {
            strengths: vec![SandboxStrength::Namespace],
            readonly_root: true,
            network_disabled: true,
            writable_paths: vec![],
        })
        .await
        .expect_err("an unimplemented sandbox must never return success");
    assert!(matches!(error.kind, HostErrorKind::Unsupported(_)));
}

#[tokio::test]
async fn service_status_distinguishes_unsupported_permission_or_missing_unit() {
    match any_service_host()
        .status("aletheon-contract-does-not-exist.service")
        .await
    {
        Ok(_) => {}
        Err(error) => assert!(matches!(
            error.kind,
            HostErrorKind::Unsupported(_)
                | HostErrorKind::PermissionDenied(_)
                | HostErrorKind::NotFound(_)
                | HostErrorKind::Io(_)
        )),
    }
}

#[test]
fn receipts_bound_persisted_detail() {
    let receipt = platform::HostReceipt::err("contract", 1, "x".repeat(100_000));
    assert!(receipt.detail.unwrap().len() <= platform::receipt::MAX_RECEIPT_DETAIL_BYTES);
}
