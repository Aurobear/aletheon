//! G3 session-owned prompt queue and safe-point interjection buffer.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use fabric::{
    evaluate_cancel, evaluate_edit, truncate_prompt_content, ConnectionId, PrincipalId,
    PromptEnvelope, PromptId, PromptKind, PromptState, QueueOpResult, QueueSnapshot, ThreadId,
    MAX_QUEUE_LEN,
};
use tokio::sync::Mutex;

#[async_trait]
pub trait PromptQueueStore: Send + Sync {
    async fn append(&self, envelope: PromptEnvelope) -> Result<PromptEnvelope>;
    async fn get(&self, id: PromptId) -> Result<Option<PromptEnvelope>>;
    async fn update(&self, envelope: PromptEnvelope) -> Result<()>;
    async fn ordered(
        &self,
        principal: &PrincipalId,
        thread: &ThreadId,
    ) -> Result<Vec<PromptEnvelope>>;
    async fn mark_consumed(&self, id: PromptId, receipt: &str) -> Result<bool>;
}

#[derive(Clone)]
struct StoredPrompt {
    sequence: u64,
    envelope: PromptEnvelope,
    consume_receipt: Option<String>,
}

#[derive(Default)]
struct InMemoryPromptState {
    next_sequence: u64,
    prompts: HashMap<PromptId, StoredPrompt>,
    idempotency: HashMap<(PrincipalId, ThreadId, String), PromptId>,
}

#[derive(Default)]
pub struct InMemoryPromptQueueStore {
    state: Mutex<InMemoryPromptState>,
}

#[async_trait]
impl PromptQueueStore for InMemoryPromptQueueStore {
    async fn append(&self, envelope: PromptEnvelope) -> Result<PromptEnvelope> {
        let mut state = self.state.lock().await;
        let idem = (
            envelope.principal_id.clone(),
            envelope.thread_id.clone(),
            envelope.idempotency_key.clone(),
        );
        if let Some(id) = state.idempotency.get(&idem) {
            return Ok(state
                .prompts
                .get(id)
                .ok_or_else(|| anyhow!("idempotency index is inconsistent"))?
                .envelope
                .clone());
        }
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        state.idempotency.insert(idem, envelope.prompt_id);
        state.prompts.insert(
            envelope.prompt_id,
            StoredPrompt {
                sequence,
                envelope: envelope.clone(),
                consume_receipt: None,
            },
        );
        Ok(envelope)
    }

    async fn get(&self, id: PromptId) -> Result<Option<PromptEnvelope>> {
        Ok(self
            .state
            .lock()
            .await
            .prompts
            .get(&id)
            .map(|record| record.envelope.clone()))
    }

    async fn update(&self, envelope: PromptEnvelope) -> Result<()> {
        let mut state = self.state.lock().await;
        let record = state
            .prompts
            .get_mut(&envelope.prompt_id)
            .ok_or_else(|| anyhow!("prompt not found"))?;
        record.envelope = envelope;
        Ok(())
    }

    async fn ordered(
        &self,
        principal: &PrincipalId,
        thread: &ThreadId,
    ) -> Result<Vec<PromptEnvelope>> {
        let state = self.state.lock().await;
        let mut records: Vec<_> = state
            .prompts
            .values()
            .filter(|record| {
                record.envelope.principal_id == *principal && record.envelope.thread_id == *thread
            })
            .collect();
        records.sort_by_key(|record| record.sequence);
        Ok(records
            .into_iter()
            .map(|record| record.envelope.clone())
            .collect())
    }

    async fn mark_consumed(&self, id: PromptId, receipt: &str) -> Result<bool> {
        let mut state = self.state.lock().await;
        let record = state
            .prompts
            .get_mut(&id)
            .ok_or_else(|| anyhow!("prompt not found"))?;
        if record.consume_receipt.is_some() {
            return Ok(false);
        }
        record.consume_receipt = Some(receipt.to_owned());
        record.envelope.state = PromptState::Completed;
        record.envelope.version = record.envelope.version.saturating_add(1);
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PromptQueueMetricSnapshot {
    pub prompt_queue_depth: u64,
    pub prompt_edit_conflict_total: u64,
    pub interjection_dropped_bytes_total: u64,
}

pub struct SessionInputCoordinator {
    store: Arc<dyn PromptQueueStore>,
    operation_lock: Mutex<()>,
    interjections: Mutex<HashMap<(PrincipalId, ThreadId), InterjectionBuffer>>,
    edit_conflicts: AtomicU64,
    dropped_interjection_bytes: AtomicU64,
}

impl SessionInputCoordinator {
    pub fn new(store: Arc<dyn PromptQueueStore>) -> Self {
        Self {
            store,
            operation_lock: Mutex::new(()),
            interjections: Mutex::new(HashMap::new()),
            edit_conflicts: AtomicU64::new(0),
            dropped_interjection_bytes: AtomicU64::new(0),
        }
    }

    pub fn in_memory() -> Self {
        Self::new(Arc::new(InMemoryPromptQueueStore::default()))
    }

    pub async fn enqueue(
        &self,
        principal: PrincipalId,
        connection: ConnectionId,
        thread: ThreadId,
        kind: PromptKind,
        content: String,
        idempotency_key: String,
    ) -> Result<PromptEnvelope> {
        let _guard = self.operation_lock.lock().await;
        let existing = self.store.ordered(&principal, &thread).await?;
        if let Some(existing) = existing
            .iter()
            .find(|prompt| prompt.idempotency_key == idempotency_key)
        {
            return Ok(existing.clone());
        }
        let depth = existing
            .iter()
            .filter(|prompt| matches!(prompt.state, PromptState::Queued | PromptState::Running))
            .count();
        if depth >= MAX_QUEUE_LEN {
            return Err(anyhow!("prompt queue capacity exceeded"));
        }
        let now = unix_now();
        let (content, dropped) = truncate_prompt_content(kind, &content);
        if kind == PromptKind::Interjection {
            self.dropped_interjection_bytes
                .fetch_add(dropped as u64, Ordering::Relaxed);
        }
        let envelope = self
            .store
            .append(PromptEnvelope {
                prompt_id: PromptId::new(),
                version: 0,
                principal_id: principal.clone(),
                connection_id: connection,
                thread_id: thread.clone(),
                kind,
                content,
                created_at_unix: now,
                updated_at_unix: now,
                state: PromptState::Queued,
                idempotency_key,
            })
            .await?;
        if kind == PromptKind::Interjection {
            self.interjections
                .lock()
                .await
                .entry((principal, thread.clone()))
                .or_insert_with(|| InterjectionBuffer::new(thread))
                .push(envelope.clone());
        }
        Ok(envelope)
    }

    pub async fn edit(
        &self,
        id: PromptId,
        expected_version: u64,
        editor: (PrincipalId, ConnectionId),
        new_content: String,
    ) -> Result<QueueOpResult> {
        let _guard = self.operation_lock.lock().await;
        let Some(mut current) = self.store.get(id).await? else {
            return Ok(QueueOpResult::Rejected {
                reason: "prompt not found".into(),
            });
        };
        let verdict = evaluate_edit(
            &current,
            &editor.0,
            editor.1.clone(),
            expected_version,
            &new_content,
        );
        if let QueueOpResult::Ok { new_version } = verdict {
            let (bounded, dropped) = truncate_prompt_content(current.kind, &new_content);
            if current.kind == PromptKind::Interjection {
                self.dropped_interjection_bytes
                    .fetch_add(dropped as u64, Ordering::Relaxed);
            }
            current.version = new_version;
            current.connection_id = editor.1;
            current.content = bounded;
            current.updated_at_unix = unix_now();
            self.store.update(current).await?;
            Ok(QueueOpResult::Ok { new_version })
        } else {
            if matches!(verdict, QueueOpResult::Conflict { .. }) {
                self.edit_conflicts.fetch_add(1, Ordering::Relaxed);
            }
            Ok(verdict)
        }
    }

    pub async fn cancel(
        &self,
        id: PromptId,
        expected_version: u64,
        requester: PrincipalId,
    ) -> Result<QueueOpResult> {
        let _guard = self.operation_lock.lock().await;
        let Some(mut current) = self.store.get(id).await? else {
            return Ok(QueueOpResult::Rejected {
                reason: "prompt not found".into(),
            });
        };
        let verdict = evaluate_cancel(&current, &requester, expected_version);
        if let QueueOpResult::Ok { new_version } = verdict {
            current.version = new_version;
            current.state = PromptState::Cancelled;
            current.updated_at_unix = unix_now();
            self.store.update(current).await?;
            Ok(QueueOpResult::Ok { new_version })
        } else {
            if matches!(verdict, QueueOpResult::Conflict { .. }) {
                self.edit_conflicts.fetch_add(1, Ordering::Relaxed);
            }
            Ok(verdict)
        }
    }

    pub async fn take_next(
        &self,
        principal: &PrincipalId,
        thread: &ThreadId,
    ) -> Result<Option<PromptEnvelope>> {
        let _guard = self.operation_lock.lock().await;
        let next = self
            .store
            .ordered(principal, thread)
            .await?
            .into_iter()
            .find(|prompt| {
                prompt.kind == PromptKind::Prompt && prompt.state == PromptState::Queued
            });
        let Some(mut next) = next else {
            return Ok(None);
        };
        next.state = PromptState::Running;
        next.version = next.version.saturating_add(1);
        next.updated_at_unix = unix_now();
        self.store.update(next.clone()).await?;
        Ok(Some(next))
    }

    pub async fn snapshot(
        &self,
        principal: &PrincipalId,
        thread: &ThreadId,
    ) -> Result<QueueSnapshot> {
        let ordered = self.store.ordered(principal, thread).await?;
        let running = ordered
            .iter()
            .find(|prompt| prompt.state == PromptState::Running)
            .map(|prompt| prompt.prompt_id);
        let pending: Vec<_> = ordered
            .into_iter()
            .filter(|prompt| prompt.state == PromptState::Queued)
            .collect();
        let queue_position = pending
            .iter()
            .enumerate()
            .map(|(position, prompt)| (prompt.prompt_id, position))
            .collect::<BTreeMap<_, _>>();
        Ok(QueueSnapshot {
            thread_id: thread.clone(),
            running,
            pending,
            queue_position,
        })
    }

    pub async fn drain_interjections_at_safe_point(
        &self,
        principal: &PrincipalId,
        thread: &ThreadId,
        receipt_prefix: &str,
    ) -> Result<Vec<String>> {
        let drained = self
            .interjections
            .lock()
            .await
            .entry((principal.clone(), thread.clone()))
            .or_insert_with(|| InterjectionBuffer::new(thread.clone()))
            .drain_envelopes();
        let mut messages = Vec::with_capacity(drained.len());
        for envelope in drained {
            let receipt = format!("{receipt_prefix}:{}", envelope.prompt_id.0);
            if self
                .store
                .mark_consumed(envelope.prompt_id, &receipt)
                .await?
            {
                messages.push(envelope.content);
            }
        }
        Ok(messages)
    }

    pub async fn metrics(
        &self,
        principal: &PrincipalId,
        thread: &ThreadId,
    ) -> Result<PromptQueueMetricSnapshot> {
        let depth = self
            .store
            .ordered(principal, thread)
            .await?
            .into_iter()
            .filter(|prompt| matches!(prompt.state, PromptState::Queued | PromptState::Running))
            .count() as u64;
        Ok(PromptQueueMetricSnapshot {
            prompt_queue_depth: depth,
            prompt_edit_conflict_total: self.edit_conflicts.load(Ordering::Relaxed),
            interjection_dropped_bytes_total: self
                .dropped_interjection_bytes
                .load(Ordering::Relaxed),
        })
    }
}

pub struct InterjectionBuffer {
    pub thread_id: ThreadId,
    pending: VecDeque<PromptEnvelope>,
    seen: HashSet<PromptId>,
}

impl InterjectionBuffer {
    pub fn new(thread_id: ThreadId) -> Self {
        Self {
            thread_id,
            pending: VecDeque::new(),
            seen: HashSet::new(),
        }
    }

    pub fn push(&mut self, envelope: PromptEnvelope) -> bool {
        if envelope.kind != PromptKind::Interjection
            || envelope.thread_id != self.thread_id
            || !self.seen.insert(envelope.prompt_id)
        {
            return false;
        }
        self.pending.push_back(envelope);
        true
    }

    pub fn drain_at_safe_point(&mut self) -> Vec<String> {
        self.drain_envelopes()
            .into_iter()
            .map(|envelope| envelope.content)
            .collect()
    }

    fn drain_envelopes(&mut self) -> Vec<PromptEnvelope> {
        self.pending.drain(..).collect()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn principal(value: &str) -> PrincipalId {
        PrincipalId(value.into())
    }

    fn connection(value: u128) -> ConnectionId {
        ConnectionId(Uuid::from_u128(value))
    }

    fn thread(value: &str) -> ThreadId {
        ThreadId(value.into())
    }

    #[tokio::test]
    async fn concurrent_enqueue_has_one_observable_total_order() {
        let coordinator = Arc::new(SessionInputCoordinator::in_memory());
        let first = coordinator.enqueue(
            principal("p"),
            connection(1),
            thread("t"),
            PromptKind::Prompt,
            "first".into(),
            "i1".into(),
        );
        let second = coordinator.enqueue(
            principal("p"),
            connection(2),
            thread("t"),
            PromptKind::Prompt,
            "second".into(),
            "i2".into(),
        );
        let (first, second) = tokio::join!(first, second);
        let first = first.unwrap();
        let second = second.unwrap();
        let snapshot = coordinator
            .snapshot(&principal("p"), &thread("t"))
            .await
            .unwrap();

        assert_eq!(snapshot.pending.len(), 2);
        let mut positions = snapshot
            .queue_position
            .values()
            .copied()
            .collect::<Vec<_>>();
        positions.sort_unstable();
        assert_eq!(positions, [0, 1]);
        assert!(snapshot.queue_position.contains_key(&first.prompt_id));
        assert!(snapshot.queue_position.contains_key(&second.prompt_id));
    }

    #[tokio::test]
    async fn edit_enforces_version_owner_and_last_editor() {
        let coordinator = SessionInputCoordinator::in_memory();
        let queued = coordinator
            .enqueue(
                principal("owner"),
                connection(1),
                thread("t"),
                PromptKind::Prompt,
                "old".into(),
                "idem".into(),
            )
            .await
            .unwrap();

        let stale = coordinator
            .edit(
                queued.prompt_id,
                9,
                (principal("owner"), connection(2)),
                "stale".into(),
            )
            .await
            .unwrap();
        assert!(matches!(stale, QueueOpResult::Conflict { .. }));
        let foreign = coordinator
            .edit(
                queued.prompt_id,
                0,
                (principal("other"), connection(2)),
                "foreign".into(),
            )
            .await
            .unwrap();
        assert!(matches!(foreign, QueueOpResult::Rejected { .. }));
        let updated = coordinator
            .edit(
                queued.prompt_id,
                0,
                (principal("owner"), connection(2)),
                "new".into(),
            )
            .await
            .unwrap();
        assert_eq!(updated, QueueOpResult::Ok { new_version: 1 });

        let stored = coordinator
            .store
            .get(queued.prompt_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.principal_id, principal("owner"));
        assert_eq!(stored.connection_id, connection(2));
        assert_eq!(stored.content, "new");
        assert_eq!(
            coordinator
                .metrics(&principal("owner"), &thread("t"))
                .await
                .unwrap()
                .prompt_edit_conflict_total,
            1
        );
    }

    #[tokio::test]
    async fn running_edit_rejected_and_cancel_checks_version() {
        let coordinator = SessionInputCoordinator::in_memory();
        let queued = coordinator
            .enqueue(
                principal("p"),
                connection(1),
                thread("t"),
                PromptKind::Prompt,
                "run".into(),
                "idem".into(),
            )
            .await
            .unwrap();
        let running = coordinator
            .take_next(&principal("p"), &thread("t"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(running.prompt_id, queued.prompt_id);
        assert!(matches!(
            coordinator
                .edit(
                    running.prompt_id,
                    running.version,
                    (principal("p"), connection(2)),
                    "edit".into()
                )
                .await
                .unwrap(),
            QueueOpResult::Rejected { .. }
        ));
        assert!(matches!(
            coordinator
                .cancel(running.prompt_id, 0, principal("p"))
                .await
                .unwrap(),
            QueueOpResult::Conflict { .. }
        ));
        assert_eq!(
            coordinator
                .cancel(running.prompt_id, running.version, principal("p"))
                .await
                .unwrap(),
            QueueOpResult::Ok {
                new_version: running.version + 1
            }
        );
    }

    #[tokio::test]
    async fn interjections_are_bounded_fifo_independent_and_idempotent() {
        let coordinator = SessionInputCoordinator::in_memory();
        let oversized = format!("{}界", "a".repeat(fabric::MAX_INTERJECTION_BYTES - 1));
        coordinator
            .enqueue(
                principal("p"),
                connection(1),
                thread("t"),
                PromptKind::Interjection,
                oversized,
                "i1".into(),
            )
            .await
            .unwrap();
        coordinator
            .enqueue(
                principal("p"),
                connection(1),
                thread("t"),
                PromptKind::Interjection,
                "second".into(),
                "i2".into(),
            )
            .await
            .unwrap();

        let drained = coordinator
            .drain_interjections_at_safe_point(&principal("p"), &thread("t"), "turn-1")
            .await
            .unwrap();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].len(), fabric::MAX_INTERJECTION_BYTES - 1);
        assert_eq!(drained[1], "second");
        assert!(coordinator
            .drain_interjections_at_safe_point(&principal("p"), &thread("t"), "turn-1",)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            coordinator
                .metrics(&principal("p"), &thread("t"))
                .await
                .unwrap()
                .interjection_dropped_bytes_total,
            "界".len() as u64
        );
    }

    #[tokio::test]
    async fn snapshots_are_partitioned_by_principal_and_thread() {
        let coordinator = SessionInputCoordinator::in_memory();
        for (p, t, text, idem) in [
            ("p1", "t1", "visible", "i1"),
            ("p1", "t2", "other-thread", "i2"),
            ("p2", "t1", "other-principal", "i3"),
        ] {
            coordinator
                .enqueue(
                    principal(p),
                    connection(1),
                    thread(t),
                    PromptKind::Prompt,
                    text.into(),
                    idem.into(),
                )
                .await
                .unwrap();
        }
        let snapshot = coordinator
            .snapshot(&principal("p1"), &thread("t1"))
            .await
            .unwrap();
        assert_eq!(snapshot.pending.len(), 1);
        assert_eq!(snapshot.pending[0].content, "visible");
    }

    #[tokio::test]
    async fn idempotency_key_deduplicates_replay() {
        let coordinator = SessionInputCoordinator::in_memory();
        let first = coordinator
            .enqueue(
                principal("p"),
                connection(1),
                thread("t"),
                PromptKind::Prompt,
                "first".into(),
                "same".into(),
            )
            .await
            .unwrap();
        let replay = coordinator
            .enqueue(
                principal("p"),
                connection(2),
                thread("t"),
                PromptKind::Prompt,
                "changed".into(),
                "same".into(),
            )
            .await
            .unwrap();
        assert_eq!(first, replay);
        assert_eq!(
            coordinator
                .snapshot(&principal("p"), &thread("t"))
                .await
                .unwrap()
                .pending
                .len(),
            1
        );
    }
}
