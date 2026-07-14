//! Inter-process communication primitives for the kernel layer.
//!
//! Provides `MessageChannel` (point-to-point mailbox between agents) and
//! `SharedScratchpad` (shared key-value store scoped to a task).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use fabric::agent::Pid;
use fabric::Clock;
use fabric::IpcMessage;

// ---------------------------------------------------------------------------
// MessageChannel — point-to-point pipe
// ---------------------------------------------------------------------------

/// Point-to-point message channel between agents (analogous to a pipe).
///
/// Each agent registers with its `Pid` and obtains a receiving half of an
/// `mpsc` channel.  Other agents send messages addressed to that `Pid`.
pub struct MessageChannel {
    inboxes: RwLock<HashMap<Pid, mpsc::Sender<IpcMessage>>>,
    buffer_size: usize,
}

impl MessageChannel {
    /// Create a new `MessageChannel` with the given per-inbox buffer size.
    pub fn new(buffer_size: usize) -> Self {
        Self {
            inboxes: RwLock::new(HashMap::new()),
            buffer_size,
        }
    }

    /// Register an agent and return its receiving inbox.
    pub async fn register(&self, pid: Pid) -> mpsc::Receiver<IpcMessage> {
        let (tx, rx) = mpsc::channel(self.buffer_size);
        self.inboxes.write().await.insert(pid, tx);
        rx
    }

    /// Remove an agent's inbox.
    pub async fn unregister(&self, pid: &Pid) {
        self.inboxes.write().await.remove(pid);
    }

    /// Send a message to the recipient identified by `msg.to`.
    pub async fn send(&self, msg: IpcMessage) -> Result<(), IpcSendError> {
        let inboxes = self.inboxes.read().await;
        match inboxes.get(&msg.to) {
            Some(tx) => tx.send(msg).await.map_err(|_| IpcSendError::RecipientGone),
            None => Err(IpcSendError::RecipientNotFound(msg.to)),
        }
    }

    /// Number of currently registered agents.
    pub async fn registered_count(&self) -> usize {
        self.inboxes.read().await.len()
    }
}

// ---------------------------------------------------------------------------
// IpcSendError
// ---------------------------------------------------------------------------

/// Errors returned by [`MessageChannel::send`].
#[derive(Debug, Clone)]
pub enum IpcSendError {
    /// No inbox is registered for the target `Pid`.
    RecipientNotFound(Pid),
    /// The inbox exists but the receiving half was dropped.
    RecipientGone,
}

impl std::fmt::Display for IpcSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RecipientNotFound(pid) => write!(f, "recipient {} not found", pid),
            Self::RecipientGone => write!(f, "recipient inbox closed"),
        }
    }
}

impl std::error::Error for IpcSendError {}

// ---------------------------------------------------------------------------
// SharedScratchpad — task-scoped KV store
// ---------------------------------------------------------------------------

/// Shared key-value scratchpad scoped to a single task.
///
/// Agents collaborating on a task can read/write arbitrary string entries.
pub struct SharedScratchpad {
    task_id: String,
    entries: RwLock<HashMap<String, ScratchpadEntry>>,
    clock: Arc<dyn Clock>,
}

/// A single entry in the scratchpad.
#[derive(Debug, Clone)]
pub struct ScratchpadEntry {
    pub key: String,
    pub value: String,
    pub written_by: Pid,
    pub timestamp_ms: u64,
}

impl SharedScratchpad {
    /// Create a new empty scratchpad for the given task.
    pub fn new(task_id: String, clock: Arc<dyn Clock>) -> Self {
        Self {
            task_id,
            entries: RwLock::new(HashMap::new()),
            clock,
        }
    }

    /// The task this scratchpad belongs to.
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Read the value for `key`, if present.
    pub async fn read(&self, key: &str) -> Option<String> {
        self.entries.read().await.get(key).map(|e| e.value.clone())
    }

    /// Write (or overwrite) `key` with `value`, attributed to `writer`.
    pub async fn write(&self, key: &str, value: String, writer: Pid) {
        let timestamp_ms = self.clock.wall_now().0 as u64;
        let entry = ScratchpadEntry {
            key: key.to_string(),
            value,
            written_by: writer,
            timestamp_ms,
        };
        self.entries.write().await.insert(key.to_string(), entry);
    }

    /// Delete `key`.  Returns `true` if the key existed.
    pub async fn delete(&self, key: &str) -> bool {
        self.entries.write().await.remove(key).is_some()
    }

    /// List all keys currently in the scratchpad.
    pub async fn list_keys(&self) -> Vec<String> {
        self.entries.read().await.keys().cloned().collect()
    }

    /// Return a snapshot of all key-value pairs.
    pub async fn snapshot(&self) -> HashMap<String, String> {
        self.entries
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect()
    }

    /// Number of entries in the scratchpad.
    pub async fn entry_count(&self) -> usize {
        self.entries.read().await.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use fabric::MessageKind;

    #[tokio::test]
    async fn test_message_channel_send_recv() {
        let ch = MessageChannel::new(8);
        let pid_a = Pid::new();
        let pid_b = Pid::new();

        let mut rx_b = ch.register(pid_b).await;

        let msg = IpcMessage::task(pid_a, pid_b, "hello".to_string(), fabric::WallTime(0));
        ch.send(msg.clone()).await.expect("send should succeed");

        let received = rx_b.recv().await.expect("should receive message");
        assert_eq!(received.from, pid_a);
        assert_eq!(received.to, pid_b);
        assert_eq!(received.payload, "hello");
        assert!(matches!(received.kind, MessageKind::Task));
    }

    #[tokio::test]
    async fn test_message_channel_recipient_not_found() {
        let ch = MessageChannel::new(8);
        let pid_a = Pid::new();
        let pid_unknown = Pid::new();

        let msg = IpcMessage::task(pid_a, pid_unknown, "ghost".to_string(), fabric::WallTime(0));
        let err = ch.send(msg).await.unwrap_err();
        assert!(matches!(err, IpcSendError::RecipientNotFound(p) if p == pid_unknown));
    }

    #[tokio::test]
    async fn test_shared_scratchpad_read_write() {
        let sp = SharedScratchpad::new("task-1".to_string(), Arc::new(TestClock::default()));
        let pid = Pid::new();

        assert_eq!(sp.read("key").await, None);

        sp.write("key", "value".to_string(), pid).await;
        assert_eq!(sp.read("key").await, Some("value".to_string()));
    }

    #[tokio::test]
    async fn test_shared_scratchpad_delete() {
        let sp = SharedScratchpad::new("task-2".to_string(), Arc::new(TestClock::default()));
        let pid = Pid::new();

        sp.write("k", "v".to_string(), pid).await;
        assert!(sp.delete("k").await);
        assert_eq!(sp.read("k").await, None);
        // Deleting again should return false.
        assert!(!sp.delete("k").await);
    }

    #[tokio::test]
    async fn test_shared_scratchpad_snapshot() {
        let sp = SharedScratchpad::new("task-3".to_string(), Arc::new(TestClock::default()));
        let pid = Pid::new();

        sp.write("a", "1".to_string(), pid).await;
        sp.write("b", "2".to_string(), pid).await;
        sp.write("c", "3".to_string(), pid).await;

        let snap = sp.snapshot().await;
        assert_eq!(snap.len(), 3);
        assert_eq!(snap.get("a").map(|s| s.as_str()), Some("1"));
        assert_eq!(snap.get("b").map(|s| s.as_str()), Some("2"));
        assert_eq!(snap.get("c").map(|s| s.as_str()), Some("3"));
    }
}
