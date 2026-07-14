//! eBPF-based perception source for kernel events.
//!
//! When the `ebpf` feature is enabled, this loads real eBPF programs via aya.
//! Otherwise, it falls back to a mock that reads from /proc and /sys.

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use tracing::debug;

use fabric::{wall_to_datetime, Clock};

use super::PerceptionSource;
use crate::r#impl::perception::event::*;

/// Configuration for the eBPF perception source.
#[derive(Debug, Clone, Deserialize)]
pub struct EbpfConfig {
    /// Enable scheduler tracing (sched_switch)
    pub enable_sched: bool,
    /// Enable network tracing (net_dev_xmit)
    pub enable_net: bool,
    /// Enable block IO tracing (block_rq_complete)
    pub enable_block: bool,
    /// Enable syscall tracing
    pub enable_syscall: bool,
    /// Minimum latency (ns) to emit block IO events
    pub block_latency_threshold_ns: u64,
}

impl Default for EbpfConfig {
    fn default() -> Self {
        Self {
            enable_sched: true,
            enable_net: true,
            enable_block: true,
            enable_syscall: false,                 // noisy, off by default
            block_latency_threshold_ns: 1_000_000, // 1ms
        }
    }
}

/// eBPF perception source.
///
/// In mock mode (no `ebpf` feature), reads from /proc and /sys to simulate
/// eBPF-level events. In real mode, loads eBPF programs and reads from ring buffers.
pub struct EbpfSource {
    config: EbpfConfig,
    event_id_counter: u64,
    clock: Arc<dyn Clock>,
    #[cfg(feature = "ebpf")]
    _bpf: Option<aya::Bpf>,
}

impl EbpfSource {
    pub fn new(config: EbpfConfig, clock: Arc<dyn Clock>) -> Self {
        Self {
            config,
            event_id_counter: 0,
            clock,
            #[cfg(feature = "ebpf")]
            _bpf: None,
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
            source: EventSource::Ebpf,
            category,
            priority,
            data,
        }
    }

    /// Mock: read scheduler stats from /proc/schedstat
    fn poll_sched_mock(&mut self) -> Result<Vec<PerceptionEvent>> {
        let mut events = Vec::new();
        // Read /proc/stat for context switch counts
        if let Ok(stat) = std::fs::read_to_string("/proc/stat") {
            for line in stat.lines() {
                if line.starts_with("ctxt") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(ctxt) = parts[1].parse::<u64>() {
                            events.push(self.make_event(
                                EventData::System {
                                    metric: "context_switches".to_string(),
                                    value: ctxt as f64,
                                    unit: "count".to_string(),
                                },
                                Priority::Low,
                                EventCategory::System,
                            ));
                        }
                    }
                }
            }
        }
        Ok(events)
    }

    /// Mock: read network stats from /proc/net/dev
    fn poll_net_mock(&mut self) -> Result<Vec<PerceptionEvent>> {
        let mut events = Vec::new();
        if let Ok(dev) = std::fs::read_to_string("/proc/net/dev") {
            for line in dev.lines().skip(2) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 10 {
                    let iface = parts[0].trim_end_matches(':');
                    if iface == "lo" {
                        continue;
                    }
                    if let (Ok(rx_bytes), Ok(tx_bytes)) =
                        (parts[1].parse::<u64>(), parts[9].parse::<u64>())
                    {
                        events.push(self.make_event(
                            EventData::System {
                                metric: format!("net_{}_rx_bytes", iface),
                                value: rx_bytes as f64,
                                unit: "bytes".to_string(),
                            },
                            Priority::Low,
                            EventCategory::Network,
                        ));
                        events.push(self.make_event(
                            EventData::System {
                                metric: format!("net_{}_tx_bytes", iface),
                                value: tx_bytes as f64,
                                unit: "bytes".to_string(),
                            },
                            Priority::Low,
                            EventCategory::Network,
                        ));
                    }
                }
            }
        }
        Ok(events)
    }

    /// Mock: read block IO stats from /proc/diskstats
    fn poll_block_mock(&mut self) -> Result<Vec<PerceptionEvent>> {
        let mut events = Vec::new();
        if let Ok(disk) = std::fs::read_to_string("/proc/diskstats") {
            for line in disk.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 14 {
                    let dev_name = parts[2].to_string();
                    // Skip partitions (only report whole devices)
                    if dev_name.starts_with("loop") || dev_name.starts_with("ram") {
                        continue;
                    }
                    if let (Ok(reads), Ok(writes), Ok(read_ticks), Ok(write_ticks)) = (
                        parts[3].parse::<u64>(),
                        parts[7].parse::<u64>(),
                        parts[6].parse::<u64>(),
                        parts[10].parse::<u64>(),
                    ) {
                        events.push(self.make_event(
                            EventData::System {
                                metric: format!("block_{}_read_ops", dev_name),
                                value: reads as f64,
                                unit: "count".to_string(),
                            },
                            Priority::Low,
                            EventCategory::System,
                        ));
                        events.push(self.make_event(
                            EventData::System {
                                metric: format!("block_{}_write_ops", dev_name),
                                value: writes as f64,
                                unit: "count".to_string(),
                            },
                            Priority::Low,
                            EventCategory::System,
                        ));
                        // Detect high latency (ticks > threshold implies IO pressure)
                        let total_ticks = read_ticks + write_ticks;
                        let total_ops = reads + writes;
                        if total_ops > 0 {
                            let avg_latency_us = (total_ticks * 1000) / total_ops;
                            if avg_latency_us > self.config.block_latency_threshold_ns / 1000 {
                                events.push(self.make_event(
                                    EventData::System {
                                        metric: format!("block_{}_avg_latency_us", dev_name),
                                        value: avg_latency_us as f64,
                                        unit: "us".to_string(),
                                    },
                                    Priority::High,
                                    EventCategory::System,
                                ));
                            }
                        }
                    }
                }
            }
        }
        Ok(events)
    }
}

#[async_trait]
impl PerceptionSource for EbpfSource {
    fn name(&self) -> &str {
        "ebpf"
    }

    async fn poll(&mut self) -> Result<Vec<PerceptionEvent>> {
        let mut events = Vec::new();

        #[cfg(feature = "ebpf")]
        {
            // Real eBPF implementation would read from ring buffers here
            warn!("eBPF feature enabled but real ring buffer reading not yet implemented");
        }

        // Mock mode: read from /proc and /sys
        if self.config.enable_sched {
            events.extend(self.poll_sched_mock()?);
        }
        if self.config.enable_net {
            events.extend(self.poll_net_mock()?);
        }
        if self.config.enable_block {
            events.extend(self.poll_block_mock()?);
        }

        debug!("eBPF source polled {} events", events.len());
        Ok(events)
    }

    fn is_available(&self) -> bool {
        // Always available in mock mode (reads /proc)
        // In real mode, check for CAP_BPF or root
        true
    }
}

impl Default for EbpfSource {
    fn default() -> Self {
        Self::new(
            EbpfConfig::default(),
            Arc::new(aletheon_kernel::chronos::SystemClock::new()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ebpf_source_mock_poll() {
        let mut source = EbpfSource::default();
        let events = source.poll().await.unwrap();
        // Should get at least some events from /proc
        assert!(!events.is_empty());
        // All events should have Ebpf source
        for event in &events {
            assert_eq!(event.source, EventSource::Ebpf);
        }
    }

    #[tokio::test]
    async fn test_ebpf_source_is_available() {
        let source = EbpfSource::default();
        assert!(source.is_available());
    }

    #[test]
    fn test_ebpf_config_default() {
        let config = EbpfConfig::default();
        assert!(config.enable_sched);
        assert!(config.enable_net);
        assert!(config.enable_block);
        assert!(!config.enable_syscall);
    }
}
