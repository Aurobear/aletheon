use std::fs;
use std::sync::{Arc, Barrier};

use mnemosyne::backends::supplemental::config::RetryPolicy;
use mnemosyne::backends::supplemental::{
    EnqueueOutcome, SupplementalDocument, SupplementalSpool, RemoteMemoryReceipt, RetryOutcome, SpoolError,
    SpoolLimits,
};
use mnemosyne::MemorySensitivity;
use rusqlite::{Connection, ErrorCode};

fn open(dir: &tempfile::TempDir, max_items: usize, max_bytes: u64) -> SupplementalSpool {
    SupplementalSpool::open(
        dir.path().join("spool.db"),
        SpoolLimits {
            max_items,
            max_bytes,
        },
    )
    .unwrap()
}

fn page(id: usize) -> SupplementalDocument {
    SupplementalDocument {
        slug: format!("aletheon/goal/{id}"),
        content: format!("---\nschema: aletheon.memory/v1\n---\n\nGoal outcome {id}"),
    }
}

#[test]
fn enqueue_is_durable_idempotent_bounded_and_sensitivity_gated() {
    let dir = tempfile::tempdir().unwrap();
    let spool = open(&dir, 1, 4096);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(spool.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert_eq!(
        spool
            .enqueue("goal-1", &page(1), MemorySensitivity::Internal, 10)
            .unwrap(),
        EnqueueOutcome::Inserted
    );
    assert_eq!(
        spool
            .enqueue("goal-1", &page(1), MemorySensitivity::Internal, 11)
            .unwrap(),
        EnqueueOutcome::AlreadyPresent
    );
    assert!(matches!(
        spool.enqueue("goal-1", &page(2), MemorySensitivity::Internal, 12),
        Err(SpoolError::Conflict)
    ));
    assert!(matches!(
        spool.enqueue("goal-2", &page(2), MemorySensitivity::Internal, 12),
        Err(SpoolError::QuotaExceeded)
    ));
    assert_eq!(
        spool
            .enqueue("secret", &page(3), MemorySensitivity::Restricted, 12)
            .unwrap(),
        EnqueueOutcome::ExcludedSensitive
    );
    let credential = SupplementalDocument {
        slug: "aletheon/goal/credential".into(),
        content: "Bearer should-not-be-spooled".into(),
    };
    assert!(matches!(
        spool.enqueue("credential", &credential, MemorySensitivity::Internal, 12),
        Err(SpoolError::Invalid(_))
    ));
    drop(spool);
    let reopened = open(&dir, 1, 4096);
    assert_eq!(reopened.queue_depth().unwrap(), 1);
}

#[test]
fn expired_lease_redelivers_and_ack_receipt_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let spool = open(&dir, 10, 4096);
    spool
        .enqueue("goal-1", &page(1), MemorySensitivity::Internal, 0)
        .unwrap();
    let first = spool.claim("worker-a", 0, 100, 1).unwrap().pop().unwrap();
    assert!(spool.claim("worker-b", 99, 100, 1).unwrap().is_empty());
    let second = spool.claim("worker-b", 100, 100, 1).unwrap().pop().unwrap();
    assert_eq!(
        first.slug, second.slug,
        "crash before ack safely redelivers stable page"
    );
    assert_eq!(second.attempt, 2);
    let first_receipt = RemoteMemoryReceipt {
        record_id: first.record_id.clone(),
        logical_page_id: first.logical_page_id.clone(),
        remote_id: "receipt-1".into(),
        content_hash: first.content_hash.clone(),
        operation: first.operation,
        schema_version: first.schema_version,
        synced_at_ms: 101,
    };
    assert!(matches!(
        spool.acknowledge(&first, "worker-a", &first_receipt),
        Err(SpoolError::LeaseMismatch)
    ));
    let receipt = RemoteMemoryReceipt {
        record_id: second.record_id.clone(),
        logical_page_id: second.logical_page_id.clone(),
        remote_id: "receipt-1".into(),
        content_hash: second.content_hash.clone(),
        operation: second.operation,
        schema_version: second.schema_version,
        synced_at_ms: 102,
    };
    spool.acknowledge(&second, "worker-b", &receipt).unwrap();
    spool.acknowledge(&second, "worker-b", &receipt).unwrap();
    assert!(spool.has_receipt("goal-1").unwrap());
    assert_eq!(spool.queue_depth().unwrap(), 0);
}

#[test]
fn concurrent_workers_claim_each_record_once() {
    let dir = tempfile::tempdir().unwrap();
    let spool = Arc::new(open(&dir, 20, 100_000));
    for id in 0..10 {
        spool
            .enqueue(
                &format!("goal-{id}"),
                &page(id),
                MemorySensitivity::Internal,
                0,
            )
            .unwrap();
    }
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for worker in ["a", "b"] {
        let spool = spool.clone();
        let barrier = barrier.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            spool.claim(worker, 0, 1000, 10).unwrap()
        }));
    }
    barrier.wait();
    let mut all = handles
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    all.sort_by(|a, b| a.record_id.cmp(&b.record_id));
    all.dedup_by(|a, b| a.record_id == b.record_id);
    assert_eq!(all.len(), 10);
}

#[test]
fn retry_backoff_dead_letter_and_requeue_are_persistent() {
    let dir = tempfile::tempdir().unwrap();
    let spool = open(&dir, 10, 100_000);
    spool
        .enqueue("goal-1", &page(1), MemorySensitivity::Internal, 0)
        .unwrap();
    spool.claim("worker", 0, 100, 1).unwrap();
    let policy = RetryPolicy {
        initial_delay_ms: 100,
        max_delay_ms: 1_000,
        max_attempts: 2,
        max_age_secs: 60,
    };
    let RetryOutcome::Scheduled { next_attempt_ms } = spool
        .retry("goal-1", "worker", "provider", 10, &policy, false)
        .unwrap()
    else {
        panic!("expected retry")
    };
    assert!((110..=135).contains(&next_attempt_ms));
    assert!(spool
        .claim("worker", next_attempt_ms - 1, 100, 1)
        .unwrap()
        .is_empty());
    spool.claim("worker", next_attempt_ms, 100, 1).unwrap();
    assert_eq!(
        spool
            .retry(
                "goal-1",
                "worker",
                "invalid_page",
                next_attempt_ms + 1,
                &policy,
                true
            )
            .unwrap(),
        RetryOutcome::DeadLettered
    );
    let dead = spool.dead_letters(10).unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].reason_category, "invalid_page");
    spool
        .requeue_dead_letter("goal-1", next_attempt_ms + 2)
        .unwrap();
    assert!(spool.dead_letters(10).unwrap().is_empty());
    assert_eq!(
        spool
            .claim("worker-2", next_attempt_ms + 2, 100, 1)
            .unwrap()[0]
            .attempt,
        1
    );
}

#[test]
fn corrupted_payload_is_quarantined() {
    let dir = tempfile::tempdir().unwrap();
    let spool = open(&dir, 10, 100_000);
    spool
        .enqueue("goal-1", &page(1), MemorySensitivity::Internal, 0)
        .unwrap();
    let connection = Connection::open(spool.path()).unwrap();
    connection
        .execute(
            "UPDATE gbrain_pages SET content='tampered' WHERE record_id='goal-1'",
            [],
        )
        .unwrap();
    assert!(matches!(
        spool.claim("worker", 0, 100, 1),
        Err(SpoolError::Corrupt)
    ));
    assert_eq!(spool.queue_depth().unwrap(), 0);
    assert_eq!(
        spool.dead_letters(1).unwrap()[0].reason_category,
        "corrupt_payload"
    );
}

#[test]
fn legacy_migration_redacts_commits_then_renames_and_restarts_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("legacy");
    fs::create_dir(&legacy).unwrap();
    let original = serde_json::json!({
        "slug":"aletheon/sessions/2026-07-15-abc",
        "markdown":"summary Bearer top-secret password=hunter2",
        "attempts":0,"next_attempt_at":0.0,"last_error":""
    });
    let source = legacy.join("abc.json");
    fs::write(&source, original.to_string()).unwrap();
    let spool = open(&dir, 10, 100_000);
    let report = spool.migrate_legacy_outbox(&legacy, 10, 0).unwrap();
    assert_eq!(report.imported, 1);
    assert!(!source.exists());
    assert!(legacy.join("abc.json.migrated").exists());
    let claimed = spool.claim("worker", 0, 100, 1).unwrap().pop().unwrap();
    assert!(claimed.content.contains("[REDACTED]"));
    assert!(!claimed.content.contains("top-secret"));
    assert!(!claimed.content.contains("hunter2"));

    // Simulate a crash after SQLite commit but before the legacy rename.
    fs::write(&source, original.to_string()).unwrap();
    let report = spool.migrate_legacy_outbox(&legacy, 10, 1).unwrap();
    assert_eq!(report.already_present, 1);
    assert!(!source.exists());
    assert_eq!(spool.queue_depth().unwrap(), 1);
}

#[test]
fn sqlite_full_is_reported_without_silent_drop() {
    let dir = tempfile::tempdir().unwrap();
    let spool = open(&dir, 100, 10_000_000);
    spool.inject_disk_full_once();
    let large = SupplementalDocument {
        slug: "aletheon/goal/full".into(),
        content: "x".repeat(100_000),
    };
    let error = spool
        .enqueue("goal-full", &large, MemorySensitivity::Internal, 0)
        .unwrap_err();
    match error {
        SpoolError::Storage(rusqlite::Error::SqliteFailure(code, _)) => {
            assert_eq!(code.code, ErrorCode::DiskFull)
        }
        other => panic!("expected SQLite full error, got {other:?}"),
    }
    assert_eq!(spool.queue_depth().unwrap(), 0);
}
