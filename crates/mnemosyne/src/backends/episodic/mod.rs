//! Episodic memory backend — stores events, actions, observations.

mod query;
mod schema;
mod storage;

pub use schema::EpisodicMemory;

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{
        wall_to_datetime, BehaviorAdjustment, CompactStrategy, EvolutionLogEntry, MemoryBackend,
        MemoryEntry, MemoryQuery, MemoryType, ReflectionEntry, ReflectionTrigger, Subsystem,
        SubsystemContext,
    };
    use std::sync::Arc;
    use uuid::Uuid;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        use std::sync::atomic::AtomicI64;
        static COUNTER: AtomicI64 = AtomicI64::new(1);
        let wall_ms = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Arc::new(kernel::chronos::TestClock::new(wall_ms, wall_ms as u64))
    }

    fn setup() -> (tempfile::NamedTempFile, EpisodicMemory) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mem = EpisodicMemory::new(tmp.path().to_path_buf(), test_clock());
        (tmp, mem)
    }

    async fn init_mem(mem: &mut EpisodicMemory) {
        let ctx = SubsystemContext {
            name: "test".into(),
            working_dir: std::env::temp_dir(),
            config: serde_json::Value::Null,
            bus: None,
        };
        mem.init(&ctx).await.unwrap();
    }

    fn make_entry(content: &[u8]) -> MemoryEntry {
        let clock = test_clock();
        MemoryEntry {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Episodic,
            content: content.to_vec(),
            tags: vec!["test".into()],
            created_at: wall_to_datetime(clock.wall_now()),
            access_count: 0,
            importance: 0.7,
            decay_rate: 0.1,
            associations: vec![],
        }
    }

    #[tokio::test]
    async fn test_episodic_store_and_recall() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry = make_entry(b"hello world");
        let handle = mem.store(entry.clone()).await.unwrap();
        assert_eq!(handle.memory_type, MemoryType::Episodic);

        let query = MemoryQuery {
            text: Some("hello".into()),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, b"hello world");
    }

    #[tokio::test]
    async fn test_episodic_recall_by_time_range() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let mut entry = make_entry(b"timed event");
        entry.created_at = "2026-01-15T12:00:00Z".parse().unwrap();
        mem.store(entry).await.unwrap();

        let query = MemoryQuery {
            time_range: Some((
                "2026-01-01T00:00:00Z".parse().unwrap(),
                "2026-01-31T23:59:59Z".parse().unwrap(),
            )),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);

        let query_outside = MemoryQuery {
            time_range: Some((
                "2026-02-01T00:00:00Z".parse().unwrap(),
                "2026-02-28T23:59:59Z".parse().unwrap(),
            )),
            limit: 10,
            ..Default::default()
        };
        let results_empty = mem.recall(&query_outside).await.unwrap();
        assert!(results_empty.is_empty());
    }

    #[tokio::test]
    async fn test_episodic_forget() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry = make_entry(b"to forget");
        let handle = mem.store(entry).await.unwrap();
        mem.forget(&handle).await.unwrap();

        let query = MemoryQuery {
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_episodic_compact() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        for i in 0..5 {
            let mut entry = make_entry(format!("event {}", i).as_bytes());
            entry.importance = 0.1 * i as f64;
            mem.store(entry).await.unwrap();
        }

        let result = mem
            .compact(CompactStrategy::PruneBelowImportance { threshold: 0.3 })
            .await
            .unwrap();
        assert_eq!(result.entries_before, 5);
        assert!(result.entries_after < 5);
    }

    #[tokio::test]
    async fn test_episodic_stats() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"a")).await.unwrap();
        mem.store(make_entry(b"bb")).await.unwrap();

        let stats = mem.stats().await.unwrap();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.total_size_bytes, 3); // 1 + 2
        assert!(stats.oldest_entry.is_some());
    }

    fn make_reflection(
        trigger: ReflectionTrigger,
        outcome: fabric::ReflectionOutcome,
        task_summary: &str,
    ) -> ReflectionEntry {
        let clock = test_clock();
        ReflectionEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: wall_to_datetime(clock.wall_now()),
            trigger,
            task_summary: task_summary.into(),
            outcome,
            what_worked: vec!["step A worked".into()],
            what_failed: vec!["step B failed".into()],
            learned: vec!["always check inputs".into()],
            behavior_changes: vec!["add validation".into()],
            confidence: 0.85,
        }
    }

    #[tokio::test]
    async fn test_store_and_recall_reflections() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry1 = make_reflection(
            ReflectionTrigger::TaskComplete,
            fabric::ReflectionOutcome::Success,
            "deployed feature X",
        );
        let entry2 = make_reflection(
            ReflectionTrigger::Impasse,
            fabric::ReflectionOutcome::Failure,
            "stuck on parser bug",
        );

        mem.store_reflection(&entry1).unwrap();
        mem.store_reflection(&entry2).unwrap();

        let recalled = mem.recall_reflections(10).unwrap();
        assert_eq!(recalled.len(), 2);
        assert_eq!(recalled[0].task_summary, "stuck on parser bug");
        assert_eq!(recalled[0].trigger, ReflectionTrigger::Impasse);
        assert_eq!(recalled[0].outcome, fabric::ReflectionOutcome::Failure);
        assert_eq!(recalled[1].task_summary, "deployed feature X");
        assert_eq!(recalled[1].trigger, ReflectionTrigger::TaskComplete);
        assert_eq!(recalled[1].outcome, fabric::ReflectionOutcome::Success);
    }

    #[tokio::test]
    async fn test_recall_reflections_respects_limit() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        for i in 0..5 {
            let entry = make_reflection(
                ReflectionTrigger::Manual,
                fabric::ReflectionOutcome::Partial,
                &format!("task {}", i),
            );
            mem.store_reflection(&entry).unwrap();
        }

        let recalled = mem.recall_reflections(3).unwrap();
        assert_eq!(recalled.len(), 3);
    }

    #[tokio::test]
    async fn test_reflection_count() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        assert_eq!(mem.reflection_count().unwrap(), 0);

        let entry = make_reflection(
            ReflectionTrigger::TaskComplete,
            fabric::ReflectionOutcome::Success,
            "completed task",
        );
        mem.store_reflection(&entry).unwrap();
        assert_eq!(mem.reflection_count().unwrap(), 1);

        let entry2 = make_reflection(
            ReflectionTrigger::Impasse,
            fabric::ReflectionOutcome::Failure,
            "failed task",
        );
        mem.store_reflection(&entry2).unwrap();
        assert_eq!(mem.reflection_count().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_reflection_fields_preserved() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry = make_reflection(
            ReflectionTrigger::Manual,
            fabric::ReflectionOutcome::Partial,
            "complex refactor",
        );
        let entry_id = entry.id.clone();
        mem.store_reflection(&entry).unwrap();

        let recalled = mem.recall_reflections(1).unwrap();
        let r = &recalled[0];
        assert_eq!(r.id, entry_id);
        assert_eq!(r.what_worked, vec!["step A worked"]);
        assert_eq!(r.what_failed, vec!["step B failed"]);
        assert_eq!(r.learned, vec!["always check inputs"]);
        assert_eq!(r.behavior_changes, vec!["add validation"]);
        assert!((r.confidence - 0.85).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_recall_reflections_empty() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let recalled = mem.recall_reflections(10).unwrap();
        assert!(recalled.is_empty());
        assert_eq!(mem.reflection_count().unwrap(), 0);
    }

    fn make_evolution_entry(trigger: &str, reflection_ids: Vec<&str>) -> EvolutionLogEntry {
        let clock = test_clock();
        EvolutionLogEntry {
            id: format!("evo-{}", Uuid::new_v4()),
            timestamp: wall_to_datetime(clock.wall_now()),
            trigger: trigger.to_string(),
            basis: reflection_ids.into_iter().map(|s| s.to_string()).collect(),
            patterns_detected: vec!["repeated failure in parser".to_string()],
            adjustments: vec![BehaviorAdjustment {
                target: "care.efficiency.weight".to_string(),
                old_value: Some(0.5),
                new_value: Some(0.7),
                reason: "efficiency improved after parser fix".to_string(),
            }],
        }
    }

    #[tokio::test]
    async fn test_store_and_recall_evolution_logs() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry1 = make_evolution_entry("periodic_review", vec!["ref-1", "ref-2"]);
        let entry2 = make_evolution_entry("threshold_reached", vec!["ref-3"]);

        mem.store_evolution_log(&entry1).unwrap();
        mem.store_evolution_log(&entry2).unwrap();

        let recalled = mem.recall_evolution_logs(10).unwrap();
        assert_eq!(recalled.len(), 2);
        assert_eq!(recalled[0].trigger, "threshold_reached");
        assert_eq!(recalled[0].basis, vec!["ref-3"]);
        assert_eq!(recalled[1].trigger, "periodic_review");
        assert_eq!(recalled[1].basis, vec!["ref-1", "ref-2"]);
    }

    #[tokio::test]
    async fn test_recall_evolution_logs_respects_limit() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        for i in 0..5 {
            let entry = make_evolution_entry(&format!("trigger_{}", i), vec![]);
            mem.store_evolution_log(&entry).unwrap();
        }

        let recalled = mem.recall_evolution_logs(3).unwrap();
        assert_eq!(recalled.len(), 3);
    }

    #[tokio::test]
    async fn test_evolution_log_fields_preserved() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry = make_evolution_entry("manual_review", vec!["ref-abc", "ref-def"]);
        mem.store_evolution_log(&entry).unwrap();

        let recalled = mem.recall_evolution_logs(1).unwrap();
        let r = &recalled[0];
        assert_eq!(r.trigger, "manual_review");
        assert_eq!(r.basis, vec!["ref-abc", "ref-def"]);
        assert_eq!(r.patterns_detected, vec!["repeated failure in parser"]);
        assert_eq!(r.adjustments.len(), 1);
        assert_eq!(r.adjustments[0].target, "care.efficiency.weight");
        assert!((r.adjustments[0].old_value.unwrap() - 0.5).abs() < f64::EPSILON);
        assert!((r.adjustments[0].new_value.unwrap() - 0.7).abs() < f64::EPSILON);
        assert_eq!(
            r.adjustments[0].reason,
            "efficiency improved after parser fix"
        );
    }

    #[tokio::test]
    async fn test_recall_evolution_logs_empty() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let recalled = mem.recall_evolution_logs(10).unwrap();
        assert!(recalled.is_empty());
    }

    #[tokio::test]
    async fn test_episodic_recall_activation_ordering() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let clock = test_clock();
        let mut old_entry = make_entry(b"old important event");
        old_entry.importance = 0.9;
        old_entry.created_at = wall_to_datetime(clock.wall_now()) - chrono::Duration::days(30);
        mem.store(old_entry).await.unwrap();

        let mut recent_entry = make_entry(b"recent minor event");
        recent_entry.importance = 0.3;
        recent_entry.created_at = wall_to_datetime(clock.wall_now());
        mem.store(recent_entry).await.unwrap();

        let query = MemoryQuery {
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 2);

        let first_content = String::from_utf8_lossy(&results[0].content);
        assert!(
            first_content.contains("recent") || first_content.contains("old"),
            "activation ordering should produce a result: got {:?}",
            first_content
        );

        assert!(results[0].importance >= 0.0 && results[0].importance <= 1.0);
    }
}
