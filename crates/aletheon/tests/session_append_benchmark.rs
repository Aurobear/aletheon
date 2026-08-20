#![cfg(feature = "test-support")]
//! Reproducible S1 scale evidence.
//!
//! Run explicitly (it is ignored in ordinary validation):
//! `bash scripts/cargo-agent.sh test -p executive --test session_append_benchmark -- --ignored --nocapture`

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use ::contracts::{
    ItemId, ItemPayload, ItemRecord, SessionId, SessionRecord, SessionStatus, TurnId,
    SESSION_SCHEMA_VERSION,
};
use adapters_sqlite::session::canonical_store::CanonicalSessionStore;
use adapters_sqlite::{event_spine::SqliteEventSpine, projection_set::DefaultEventProjectionSet};
use rusqlite::{params, Connection};

fn percentile(sorted_micros: &[u128], percentile: usize) -> u128 {
    let index = (sorted_micros.len() - 1) * percentile / 100;
    sorted_micros[index]
}

fn summary(mut micros: Vec<u128>, elapsed_micros: u128) -> serde_json::Value {
    micros.sort_unstable();
    serde_json::json!({
        "samples": micros.len(),
        "p50_us": percentile(&micros, 50),
        "p95_us": percentile(&micros, 95),
        "p99_us": percentile(&micros, 99),
        "throughput_per_second": (micros.len() as f64 * 1_000_000.0) / elapsed_micros.max(1) as f64,
    })
}

fn seed(path: &std::path::Path, total_events: usize, session_count: usize) {
    let store = CanonicalSessionStore::open(path).unwrap();
    drop(store);
    let mut connection = Connection::open(path).unwrap();
    let transaction = connection.transaction().unwrap();
    let mut insert_session = transaction
        .prepare(
            "INSERT INTO sessions(session_id,schema_version,record_json,next_sequence) \
             VALUES(?1,?2,?3,?4)",
        )
        .unwrap();
    let mut insert_item = transaction
        .prepare(
            "INSERT INTO session_items(session_id,sequence,item_id,turn_id,item_json) \
             VALUES(?1,?2,?3,?4,?5)",
        )
        .unwrap();
    let base = total_events / session_count;
    let remainder = total_events % session_count;
    for index in 0..session_count {
        let session_id = SessionId(format!("bench-{index}"));
        let count = base + usize::from(index < remainder);
        let session = SessionRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: session_id.clone(),
            parent: None,
            created_at_ms: 1,
            status: SessionStatus::Active,
        };
        insert_session
            .execute(params![
                session_id.0,
                SESSION_SCHEMA_VERSION,
                serde_json::to_string(&session).unwrap(),
                count as u64 + 1,
            ])
            .unwrap();
        let turn_id = TurnId::new();
        for sequence in 1..=count as u64 {
            let item = ItemRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: ItemId::new(),
                session_id: session_id.clone(),
                turn_id,
                sequence,
                created_at_ms: sequence,
                payload: ItemPayload::SystemNotice {
                    content: "seed".into(),
                },
            };
            insert_item
                .execute(params![
                    session_id.0,
                    sequence,
                    item.id.0.to_string(),
                    turn_id.0.to_string(),
                    serde_json::to_string(&item).unwrap(),
                ])
                .unwrap();
        }
    }
    drop(insert_item);
    drop(insert_session);
    transaction.commit().unwrap();
}

#[tokio::test]
#[ignore = "scale evidence; run explicitly for S1 acceptance"]
async fn s1_append_scale_matrix_reports_percentiles_and_throughput() {
    let mut reports = Vec::new();
    for total_events in [1_000usize, 10_000, 100_000] {
        for session_count in [1usize, 10, 100] {
            let directory = tempfile::tempdir().unwrap();
            let session_path = directory.path().join("sessions.db");
            seed(&session_path, total_events, session_count);
            let store = aletheon::host::session::test_composition::compose_session_store(
                Arc::new(CanonicalSessionStore::open(&session_path).unwrap()),
                Arc::new(SqliteEventSpine::open(directory.path().join("events.db")).unwrap()),
                Arc::new(DefaultEventProjectionSet::in_memory()),
            );
            let mut heads = HashMap::new();
            let base = total_events / session_count;
            let remainder = total_events % session_count;
            for index in 0..session_count {
                let session_id = SessionId(format!("bench-{index}"));
                heads.insert(
                    session_id,
                    (base + usize::from(index < remainder)) as u64 + 1,
                );
            }

            let mut optimized = Vec::new();
            let optimized_started = Instant::now();
            for sample in 0..128usize {
                let session_id = SessionId(format!("bench-{}", sample % session_count));
                let sequence = *heads.get(&session_id).unwrap();
                let started = Instant::now();
                store
                    .append(
                        &session_id,
                        sequence,
                        ItemRecord {
                            schema_version: SESSION_SCHEMA_VERSION,
                            id: ItemId::new(),
                            session_id: session_id.clone(),
                            turn_id: TurnId::new(),
                            sequence,
                            created_at_ms: sequence,
                            payload: ItemPayload::SystemNotice {
                                content: "measured".into(),
                            },
                        },
                    )
                    .await
                    .unwrap();
                optimized.push(started.elapsed().as_micros());
                heads.insert(session_id, sequence + 1);
            }
            let optimized_elapsed = optimized_started.elapsed().as_micros();

            // Reproduce the removed admission behavior by loading the complete
            // session history immediately before an append. This is a modeled
            // baseline, explicitly distinguished from historical measurements.
            let mut full_scan_baseline = Vec::new();
            let baseline_started = Instant::now();
            for sample in 0..16usize {
                let session_id = SessionId(format!("bench-{}", sample % session_count));
                let sequence = *heads.get(&session_id).unwrap();
                let started = Instant::now();
                let _history = store.load_items(&session_id, None).await.unwrap();
                store
                    .append(
                        &session_id,
                        sequence,
                        ItemRecord {
                            schema_version: SESSION_SCHEMA_VERSION,
                            id: ItemId::new(),
                            session_id: session_id.clone(),
                            turn_id: TurnId::new(),
                            sequence,
                            created_at_ms: sequence,
                            payload: ItemPayload::SystemNotice {
                                content: "baseline".into(),
                            },
                        },
                    )
                    .await
                    .unwrap();
                full_scan_baseline.push(started.elapsed().as_micros());
                heads.insert(session_id, sequence + 1);
            }
            let baseline_elapsed = baseline_started.elapsed().as_micros();

            reports.push(serde_json::json!({
                "total_seed_events": total_events,
                "sessions": session_count,
                "optimized_head_append": summary(optimized, optimized_elapsed),
                "modeled_full_history_admission": summary(full_scan_baseline, baseline_elapsed),
            }));
        }
    }
    println!("{}", serde_json::to_string_pretty(&reports).unwrap());
}
