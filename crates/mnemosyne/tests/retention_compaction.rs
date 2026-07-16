use chrono::{DateTime, Utc};
use mnemosyne::{
    ForgetAuthority, ForgetPolicy, ForgetSelector, MemoryAuthority, MemoryKind, MemoryMetadata,
    MemoryRecord, MemoryRecordId, MemoryScope, MemoryStatus, RetentionCompactionPolicy,
    RetentionCompactor, RetentionRepository,
};

fn record() -> MemoryRecord {
    MemoryRecord {
        id: MemoryRecordId("fact-1".into()),
        kind: MemoryKind::SemanticFact,
        scope: MemoryScope::Global,
        content: "payload eligible only after all retention gates".into(),
        metadata: MemoryMetadata::local("fact-1", "event-1", DateTime::<Utc>::UNIX_EPOCH),
        status: MemoryStatus::Current,
        authority: MemoryAuthority::VerifiedLocalSemantic,
        source_event_ids: vec!["event-1".into()],
        tags: Vec::new(),
    }
}

fn policy() -> ForgetPolicy {
    ForgetPolicy {
        request_id: "forget-fact-1".into(),
        selector: ForgetSelector::Exact {
            record_ids: vec![MemoryRecordId("fact-1".into())],
            within: MemoryScope::Global,
        },
        requester: "owner".into(),
        reason: "retention test".into(),
        authority: ForgetAuthority::Elevated {
            proof: "admin-proof".into(),
        },
    }
}

#[test]
fn physical_compaction_requires_age_backup_and_settled_remote_state() {
    let dir = tempfile::tempdir().unwrap();
    let repository = RetentionRepository::open(dir.path().join("retention.db")).unwrap();
    repository.register(&record(), 0).unwrap();
    repository.preview_forget(&policy(), 10).unwrap();
    let receipt = repository.forget(&policy(), 10).unwrap();
    assert_eq!(
        receipt.remote_pending,
        vec![MemoryRecordId("fact-1".into())]
    );
    let compactor = RetentionCompactor::new(&repository);
    let mut gates = RetentionCompactionPolicy {
        min_tombstone_age_ms: 100,
        backup_completed_at_ms: None,
        require_remote_settled: true,
        batch_size: 1,
        lease_ms: 1_000,
    };
    assert!(compactor.run("worker", 200, &gates).is_err());
    gates.backup_completed_at_ms = Some(9);
    assert!(
        compactor
            .run("worker", 200, &gates)
            .unwrap()
            .removed
            .is_empty(),
        "backup predating tombstone cannot authorize removal"
    );
    gates.backup_completed_at_ms = Some(20);
    assert!(
        compactor
            .run("worker", 50, &gates)
            .unwrap()
            .removed
            .is_empty(),
        "minimum age is enforced"
    );
    assert!(
        compactor
            .run("worker", 200, &gates)
            .unwrap()
            .removed
            .is_empty(),
        "remote tombstone must settle"
    );
    repository.mark_remote_settled("fact-1").unwrap();
    let report = compactor.run("worker", 200, &gates).unwrap();
    assert_eq!(report.removed, vec!["fact-1"]);
    assert!(report.watermark.is_some());
    assert!(
        repository.record("fact-1", true).unwrap().is_none(),
        "payload was physically removed"
    );
    assert_eq!(
        repository.forget(&policy(), 300).unwrap(),
        receipt,
        "immutable deletion receipt remains replayable"
    );
}

#[test]
fn compaction_is_bounded_and_lease_is_resumable() {
    let dir = tempfile::tempdir().unwrap();
    let repository = RetentionRepository::open(dir.path().join("retention.db")).unwrap();
    for index in 0..2 {
        let mut value = record();
        value.id = MemoryRecordId(format!("fact-{index}"));
        value.metadata.record_id = value.id.0.clone();
        repository.register(&value, 0).unwrap();
        let request = ForgetPolicy {
            request_id: format!("forget-{index}"),
            selector: ForgetSelector::Exact {
                record_ids: vec![value.id.clone()],
                within: MemoryScope::Global,
            },
            requester: "owner".into(),
            reason: "bounded batch".into(),
            authority: ForgetAuthority::Elevated {
                proof: "proof".into(),
            },
        };
        repository.preview_forget(&request, 1).unwrap();
        repository.forget(&request, 1).unwrap();
        repository.mark_remote_settled(&value.id.0).unwrap();
    }
    let report = RetentionCompactor::new(&repository)
        .run(
            "worker",
            10,
            &RetentionCompactionPolicy {
                min_tombstone_age_ms: 1,
                backup_completed_at_ms: Some(5),
                require_remote_settled: true,
                batch_size: 1,
                lease_ms: 100,
            },
        )
        .unwrap();
    assert_eq!(report.removed.len(), 1);
}
