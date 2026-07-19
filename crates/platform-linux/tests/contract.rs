//! H1-08: shared contract suite — every platform backend must pass these.

use platform_api::filesystem::{AtomicWrite, FilesystemHost};
use platform_api::path::HostPath;
use platform_api::process::{ProcessHost, ProcessSignal, SpawnSpec};

fn any_filesystem_host() -> Box<dyn FilesystemHost> {
    Box::new(platform_linux::LinuxFilesystemHost::new())
}

fn any_process_host() -> Box<dyn ProcessHost> {
    Box::new(platform_linux::LinuxProcessHost::new())
}

#[tokio::test]
async fn filesystem_write_read_round_trip() {
    let host = any_filesystem_host();
    let tmp = tempfile::tempdir().unwrap();
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

#[tokio::test]
async fn stale_write_is_rejected() {
    let host = any_filesystem_host();
    let tmp = tempfile::tempdir().unwrap();
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
