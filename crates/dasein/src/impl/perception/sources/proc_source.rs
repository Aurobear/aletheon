use async_trait::async_trait;
use std::sync::Arc;

use fabric::{wall_to_datetime, Clock};

use super::PerceptionSource;
use crate::r#impl::perception::event::*;

/// Polls /proc for system status changes.
pub struct ProcSource {
    last_load: Option<(f64, f64, f64)>,
    last_mem: Option<(u64, u64)>,
    /// Reserved for future per-core-CPU threshold gating; threshold
    /// is set at construction but not yet referenced by `poll()`.
    #[allow(dead_code)]
    high_cpu_threshold: f64,
    event_id_counter: u64,
    clock: Arc<dyn Clock>,
}

impl ProcSource {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            last_load: None,
            last_mem: None,
            high_cpu_threshold: 80.0,
            event_id_counter: 0,
            clock,
        }
    }

    fn next_id(&mut self) -> u64 {
        self.event_id_counter += 1;
        self.event_id_counter
    }

    fn make_event(
        &mut self,
        data: EventData,
        priority: Priority,
        category: EventCategory,
    ) -> PerceptionEvent {
        PerceptionEvent {
            id: self.next_id(),
            timestamp: wall_to_datetime(self.clock.wall_now()),
            source: EventSource::Proc,
            category,
            priority,
            data,
        }
    }
}

impl Default for ProcSource {
    fn default() -> Self {
        Self::new(Arc::new(aletheon_kernel::chronos::SystemClock::new()))
    }
}

#[async_trait]
impl PerceptionSource for ProcSource {
    fn name(&self) -> &str {
        "proc"
    }

    async fn poll(&mut self) -> anyhow::Result<Vec<PerceptionEvent>> {
        let mut events = Vec::new();

        // Check load average
        if let Ok(loadavg) = tokio::fs::read_to_string("/proc/loadavg").await {
            let parts: Vec<&str> = loadavg.split_whitespace().collect();
            if parts.len() >= 3 {
                if let (Ok(l1), Ok(l5), Ok(l15)) = (
                    parts[0].parse::<f64>(),
                    parts[1].parse::<f64>(),
                    parts[2].parse::<f64>(),
                ) {
                    let new_load = (l1, l5, l15);
                    if self.last_load != Some(new_load) {
                        // Only emit if load is significant (1-min > 2.0)
                        if l1 > 2.0 {
                            events.push(self.make_event(
                                EventData::LoadAvg {
                                    load1: l1,
                                    load5: l5,
                                    load15: l15,
                                },
                                if l1 > 8.0 {
                                    Priority::Critical
                                } else {
                                    Priority::Normal
                                },
                                EventCategory::System,
                            ));
                        }
                        self.last_load = Some(new_load);
                    }
                }
            }
        }

        // Check memory pressure
        if let Ok(meminfo) = tokio::fs::read_to_string("/proc/meminfo").await {
            let mut total_kb = 0u64;
            let mut available_kb = 0u64;
            for line in meminfo.lines() {
                if line.starts_with("MemTotal:") {
                    total_kb = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                } else if line.starts_with("MemAvailable:") {
                    available_kb = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                }
            }
            if total_kb > 0 {
                let total_mb = total_kb / 1024;
                let available_mb = available_kb / 1024;
                let usage_pct = 1.0 - (available_mb as f64 / total_mb as f64);

                let new_mem = (total_mb, available_mb);
                if self.last_mem != Some(new_mem) && usage_pct > 0.85 {
                    events.push(self.make_event(
                        EventData::MemoryPressure {
                            available_mb,
                            total_mb,
                        },
                        if usage_pct > 0.95 {
                            Priority::Critical
                        } else {
                            Priority::High
                        },
                        EventCategory::System,
                    ));
                }
                self.last_mem = Some(new_mem);
            }
        }

        Ok(events)
    }

    fn is_available(&self) -> bool {
        std::path::Path::new("/proc/meminfo").exists()
    }
}
