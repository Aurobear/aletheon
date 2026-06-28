use std::collections::BinaryHeap;
use std::cmp::Ordering;
use aletheon_abi::ipc_types::AgentMessage;

/// Wrapper for AgentMessage that orders by priority (lower priority value = higher precedence).
struct PriorityEntry {
    message: AgentMessage,
    sequence: u64,  // For FIFO within same priority
}

impl PartialEq for PriorityEntry {
    fn eq(&self, other: &Self) -> bool {
        self.message.priority == other.message.priority && self.sequence == other.sequence
    }
}

impl Eq for PriorityEntry {}

impl PartialOrd for PriorityEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PriorityEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse priority: lower value = higher precedence
        // BinaryHeap is a max-heap, so we reverse to get min-heap behavior
        // For same priority, lower sequence = higher precedence (FIFO)
        // Both comparisons reversed for max-heap to behave as min-heap
        other.message.priority.cmp(&self.message.priority)
            .then(other.sequence.cmp(&self.sequence))
    }
}

/// Thread-safe priority queue for agent messages.
pub struct PriorityQueue {
    heap: BinaryHeap<PriorityEntry>,
    next_sequence: u64,
    max_capacity: usize,
}

impl PriorityQueue {
    pub fn new(max_capacity: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(max_capacity.min(1024)),
            next_sequence: 0,
            max_capacity,
        }
    }

    /// Push a message into the queue. Returns false if queue is full.
    pub fn push(&mut self, message: AgentMessage) -> bool {
        if self.heap.len() >= self.max_capacity {
            // Evict lowest priority message if new one is higher priority
            if let Some(lowest) = self.heap.peek() {
                if message.priority < lowest.message.priority {
                    self.heap.pop();
                } else {
                    return false; // New message has lower or equal priority
                }
            }
        }

        self.heap.push(PriorityEntry {
            message,
            sequence: self.next_sequence,
        });
        self.next_sequence += 1;
        true
    }

    /// Pop the highest priority message.
    pub fn pop(&mut self) -> Option<AgentMessage> {
        self.heap.pop().map(|entry| entry.message)
    }

    /// Peek at the highest priority message without removing.
    pub fn peek(&self) -> Option<&AgentMessage> {
        self.heap.peek().map(|entry| &entry.message)
    }

    /// Current queue length.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Check if queue is empty.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Drain all messages in priority order.
    pub fn drain(&mut self) -> Vec<AgentMessage> {
        let mut result = Vec::with_capacity(self.heap.len());
        while let Some(msg) = self.pop() {
            result.push(msg);
        }
        result
    }

    /// Drain up to max messages.
    pub fn drain_max(&mut self, max: usize) -> Vec<AgentMessage> {
        let mut result = Vec::with_capacity(max.min(self.heap.len()));
        for _ in 0..max {
            if let Some(msg) = self.pop() {
                result.push(msg);
            } else {
                break;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::ipc_types::{MessageType, AgentId, IpcPriority};

    fn make_msg(sender: AgentId, priority: IpcPriority) -> AgentMessage {
        AgentMessage::new(sender, 0, MessageType::Event, priority, vec![0; 8])
    }

    #[test]
    fn test_priority_ordering() {
        let mut pq = PriorityQueue::new(100);
        pq.push(make_msg(1, IpcPriority::Background));
        pq.push(make_msg(2, IpcPriority::Urgent));
        pq.push(make_msg(3, IpcPriority::ToolCall));

        assert_eq!(pq.pop().unwrap().priority, IpcPriority::Urgent);
        assert_eq!(pq.pop().unwrap().priority, IpcPriority::ToolCall);
        assert_eq!(pq.pop().unwrap().priority, IpcPriority::Background);
    }

    #[test]
    fn test_fifo_within_same_priority() {
        let mut pq = PriorityQueue::new(100);
        pq.push(make_msg(1, IpcPriority::ToolCall));
        pq.push(make_msg(2, IpcPriority::ToolCall));
        pq.push(make_msg(3, IpcPriority::ToolCall));

        assert_eq!(pq.pop().unwrap().sender_id, 1);
        assert_eq!(pq.pop().unwrap().sender_id, 2);
        assert_eq!(pq.pop().unwrap().sender_id, 3);
    }

    #[test]
    fn test_capacity_eviction() {
        let mut pq = PriorityQueue::new(3);
        assert!(pq.push(make_msg(1, IpcPriority::Batch)));
        assert!(pq.push(make_msg(2, IpcPriority::Background)));
        assert!(pq.push(make_msg(3, IpcPriority::ToolCall)));
        // Queue full, try to add lower priority
        assert!(!pq.push(make_msg(4, IpcPriority::Batch)));
        // But higher priority should evict
        assert!(pq.push(make_msg(5, IpcPriority::Urgent)));
        assert_eq!(pq.len(), 3);
        assert_eq!(pq.pop().unwrap().sender_id, 5); // Urgent
    }

    #[test]
    fn test_drain() {
        let mut pq = PriorityQueue::new(100);
        pq.push(make_msg(1, IpcPriority::Background));
        pq.push(make_msg(2, IpcPriority::Urgent));
        pq.push(make_msg(3, IpcPriority::ToolCall));

        let drained = pq.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].priority, IpcPriority::Urgent);
        assert_eq!(drained[1].priority, IpcPriority::ToolCall);
        assert_eq!(drained[2].priority, IpcPriority::Background);
        assert!(pq.is_empty());
    }
}
