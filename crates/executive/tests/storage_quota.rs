use executive::r#impl::storage_quota::{
    QuotaError, StorageClass, StorageLimit, StorageQuota, StorageRoot,
};
use std::collections::HashMap;
use std::sync::{Arc, Barrier};

fn quota(root: &std::path::Path, hard: u64) -> StorageQuota {
    StorageQuota::new(HashMap::from([(
        StorageClass::Artifacts,
        StorageRoot {
            path: root.to_path_buf(),
            limit: StorageLimit {
                soft_bytes: hard / 2,
                hard_bytes: hard,
                hard_items: 4,
            },
        },
    )]))
    .unwrap()
}

#[test]
fn concurrent_reservations_cannot_overcommit() {
    let directory = tempfile::tempdir().unwrap();
    let quota = Arc::new(quota(directory.path(), 100));
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let quota = quota.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            let result = quota.reserve(StorageClass::Artifacts, 60, 1);
            barrier.wait();
            result
        }));
    }
    barrier.wait();
    barrier.wait();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
}

#[test]
fn dropped_reservation_releases_capacity() {
    let directory = tempfile::tempdir().unwrap();
    let quota = quota(directory.path(), 100);
    let reservation = quota.reserve(StorageClass::Artifacts, 80, 1).unwrap();
    assert_eq!(
        quota.reserve(StorageClass::Artifacts, 30, 1).unwrap_err(),
        QuotaError::HardLimit {
            class: StorageClass::Artifacts
        }
    );
    drop(reservation);
    quota.reserve(StorageClass::Artifacts, 30, 1).unwrap();
}

#[test]
fn existing_bytes_items_and_soft_limit_are_counted() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("one"), vec![0_u8; 51]).unwrap();
    let quota = quota(directory.path(), 100);
    assert!(quota.is_soft_limited(StorageClass::Artifacts).unwrap());
    assert_eq!(
        quota.reserve(StorageClass::Artifacts, 50, 1).unwrap_err(),
        QuotaError::HardLimit {
            class: StorageClass::Artifacts
        }
    );
}

#[cfg(unix)]
#[test]
fn symlinks_and_hardlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    symlink(outside.path(), directory.path().join("link")).unwrap();
    let quota = quota(directory.path(), 100);
    assert_eq!(
        quota.usage(StorageClass::Artifacts),
        Err(QuotaError::UnsafeEntry)
    );

    std::fs::remove_file(directory.path().join("link")).unwrap();
    std::fs::write(directory.path().join("first"), b"data").unwrap();
    std::fs::hard_link(
        directory.path().join("first"),
        directory.path().join("second"),
    )
    .unwrap();
    assert_eq!(
        quota.usage(StorageClass::Artifacts),
        Err(QuotaError::UnsafeEntry)
    );
}
