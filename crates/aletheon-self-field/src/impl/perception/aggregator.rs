use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

use super::event::*;

/// Aggregates, deduplicates, and rate-limits perception events.
pub struct EventAggregator {
    /// Content hash dedup — same hash within window = skip
    seen_hashes: HashMap<u64, std::time::Instant>,
    dedup_window: std::time::Duration,

    /// Rate limiting per source
    source_counts: HashMap<EventSource, VecDeque<std::time::Instant>>,
    max_per_source_per_second: usize,

    /// Priority boost for repeated events
    boost_counts: HashMap<String, u32>,
}

impl EventAggregator {
    pub fn new() -> Self {
        Self {
            seen_hashes: HashMap::new(),
            dedup_window: std::time::Duration::from_secs(10),
            source_counts: HashMap::new(),
            max_per_source_per_second: 50,
            boost_counts: HashMap::new(),
        }
    }

    /// Process a batch of events: deduplicate, rate-limit, boost priority.
    pub fn aggregate(&mut self, events: Vec<PerceptionEvent>) -> Vec<PerceptionEvent> {
        let now = std::time::Instant::now();
        let mut result = Vec::new();

        // Clean old hashes
        self.seen_hashes.retain(|_, t| now.duration_since(*t) < self.dedup_window);

        for mut event in events {
            // Content hash dedup
            let hash = self.content_hash(&event);
            if self.seen_hashes.contains_key(&hash) {
                continue;
            }
            self.seen_hashes.insert(hash, now);

            // Rate limiting
            let source_queue = self.source_counts
                .entry(event.source)
                .or_insert_with(VecDeque::new);

            // Remove old entries
            while let Some(front) = source_queue.front() {
                if now.duration_since(*front) > std::time::Duration::from_secs(1) {
                    source_queue.pop_front();
                } else {
                    break;
                }
            }

            if source_queue.len() >= self.max_per_source_per_second {
                continue; // Rate limited
            }
            source_queue.push_back(now);

            // Priority boost for repeated similar events
            let key = self.event_key(&event);
            let count = self.boost_counts.entry(key).or_insert(0);
            *count += 1;
            if *count >= 3 && event.priority < Priority::High {
                event.priority = match event.priority {
                    Priority::Low => Priority::Normal,
                    Priority::Normal => Priority::High,
                    other => other,
                };
            }

            result.push(event);
        }

        result
    }

    fn content_hash(&self, event: &PerceptionEvent) -> u64 {
        let mut hasher = DefaultHasher::new();
        event.source.hash(&mut hasher);
        event.category.hash(&mut hasher);
        event.summary().hash(&mut hasher);
        hasher.finish()
    }

    fn event_key(&self, event: &PerceptionEvent) -> String {
        format!("{:?}:{:?}", event.source, event.category)
    }
}

impl Default for EventAggregator {
    fn default() -> Self {
        Self::new()
    }
}
