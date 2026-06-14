//! System bottleneck detection and upgrade path recommendation.
//!
//! Monitors CPU, memory, IO, and network metrics over time,
//! detects bottleneck patterns, and generates upgrade suggestions.

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::{debug, info, warn};

use super::PerceptionSource;
use crate::r#impl::perception::event::*;

/// Maximum history entries to keep (5 minutes at 1s polling)
const MAX_HISTORY: usize = 300;

/// Bottleneck detection thresholds.
#[derive(Debug, Clone, Deserialize)]
pub struct BottleneckThreshold {
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub disk_io_latency_us: u64,
    pub network_utilization_percent: f64,
}

impl Default for BottleneckThreshold {
    fn default() -> Self {
        Self {
            cpu_percent: 90.0,
            memory_percent: 85.0,
            disk_io_latency_us: 10_000, // 10ms
            network_utilization_percent: 80.0,
        }
    }
}

/// Category of detected bottleneck.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BottleneckCategory {
    Cpu,
    Memory,
    Io,
    Network,
}

/// Severity of a bottleneck.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Recommended action for addressing a bottleneck.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpgradeSuggestion {
    EbpfOptimization {
        program: String,
        description: String,
    },
    KernelModule {
        module: String,
        reason: String,
    },
    KernelSourceChange {
        subsystem: String,
        description: String,
    },
    HardwareUpgrade {
        component: String,
        specification: String,
    },
}

/// A detected bottleneck report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleneckReport {
    pub category: BottleneckCategory,
    pub severity: Severity,
    pub current_value: f64,
    pub threshold: f64,
    pub suggestion: UpgradeSuggestion,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// System metrics snapshot.
#[derive(Debug, Clone)]
struct SystemMetrics {
    cpu_percent: f64,
    memory_percent: f64,
    disk_read_ops: u64,
    disk_write_ops: u64,
    disk_latency_us: u64,
    net_rx_bytes: u64,
    net_tx_bytes: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
}

/// Bottleneck detector perception source.
pub struct BottleneckDetector {
    threshold: BottleneckThreshold,
    history: VecDeque<SystemMetrics>,
    event_id_counter: u64,
    reports: Vec<BottleneckReport>,
}

impl BottleneckDetector {
    pub fn new(threshold: BottleneckThreshold) -> Self {
        Self {
            threshold,
            history: VecDeque::with_capacity(MAX_HISTORY),
            event_id_counter: 0,
            reports: Vec::new(),
        }
    }

    fn next_id(&mut self) -> u64 {
        self.event_id_counter += 1;
        self.event_id_counter
    }

    fn make_event(&mut self, data: EventData, priority: Priority, category: EventCategory) -> PerceptionEvent {
        PerceptionEvent {
            id: self.next_id(),
            timestamp: Utc::now(),
            source: EventSource::Proc,
            category,
            priority,
            data,
        }
    }

    /// Collect current system metrics from /proc.
    fn collect_metrics(&self) -> Result<SystemMetrics> {
        // CPU from /proc/stat
        let cpu_percent = self.read_cpu_percent().unwrap_or(0.0);

        // Memory from /proc/meminfo
        let memory_percent = self.read_memory_percent().unwrap_or(0.0);

        // Disk from /proc/diskstats
        let (disk_read_ops, disk_write_ops, disk_latency_us) = self.read_disk_stats().unwrap_or((0, 0, 0));

        // Network from /proc/net/dev
        let (net_rx_bytes, net_tx_bytes) = self.read_net_stats().unwrap_or((0, 0));

        Ok(SystemMetrics {
            cpu_percent,
            memory_percent,
            disk_read_ops,
            disk_write_ops,
            disk_latency_us,
            net_rx_bytes,
            net_tx_bytes,
            timestamp: Utc::now(),
        })
    }

    fn read_cpu_percent(&self) -> Result<f64> {
        let stat = std::fs::read_to_string("/proc/stat")?;
        let line = stat.lines().next().ok_or_else(|| anyhow::anyhow!("Empty /proc/stat"))?;
        let parts: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();

        if parts.len() >= 4 {
            let idle = parts[3];
            let total: u64 = parts.iter().sum();
            // Simple instantaneous estimate (proper impl would diff with previous)
            Ok((1.0 - (idle as f64 / total as f64)) * 100.0)
        } else {
            Ok(0.0)
        }
    }

    fn read_memory_percent(&self) -> Result<f64> {
        let meminfo = std::fs::read_to_string("/proc/meminfo")?;
        let mut total = 0u64;
        let mut available = 0u64;

        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                total = line.split_whitespace().nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            } else if line.starts_with("MemAvailable:") {
                available = line.split_whitespace().nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
        }

        if total > 0 {
            Ok((1.0 - (available as f64 / total as f64)) * 100.0)
        } else {
            Ok(0.0)
        }
    }

    fn read_disk_stats(&self) -> Result<(u64, u64, u64)> {
        let diskstats = std::fs::read_to_string("/proc/diskstats")?;
        let mut total_reads = 0u64;
        let mut total_writes = 0u64;
        let mut total_ticks = 0u64;

        for line in diskstats.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 14 {
                let dev = parts[2];
                // Skip loop/ram devices
                if dev.starts_with("loop") || dev.starts_with("ram") {
                    continue;
                }
                total_reads += parts[3].parse::<u64>().unwrap_or(0);
                total_writes += parts[7].parse::<u64>().unwrap_or(0);
                total_ticks += parts[12].parse::<u64>().unwrap_or(0); // io_ticks
            }
        }

        let total_ops = total_reads + total_writes;
        let avg_latency = if total_ops > 0 {
            (total_ticks * 1000) / total_ops // Convert ms ticks to us
        } else {
            0
        };

        Ok((total_reads, total_writes, avg_latency))
    }

    fn read_net_stats(&self) -> Result<(u64, u64)> {
        let net_dev = std::fs::read_to_string("/proc/net/dev")?;
        let mut total_rx = 0u64;
        let mut total_tx = 0u64;

        for line in net_dev.lines().skip(2) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 10 {
                let iface = parts[0].trim_end_matches(':');
                if iface == "lo" { continue; }
                total_rx += parts[1].parse::<u64>().unwrap_or(0);
                total_tx += parts[9].parse::<u64>().unwrap_or(0);
            }
        }

        Ok((total_rx, total_tx))
    }

    /// Analyze metrics history and detect bottlenecks.
    fn detect_bottlenecks(&mut self, current: &SystemMetrics) -> Vec<BottleneckReport> {
        let mut reports = Vec::new();

        // CPU bottleneck
        if current.cpu_percent > self.threshold.cpu_percent {
            let severity = if current.cpu_percent > 95.0 {
                Severity::Critical
            } else if current.cpu_percent > 90.0 {
                Severity::High
            } else {
                Severity::Medium
            };

            // Check if sustained (need at least 3 samples)
            if self.history.len() >= 3 {
                let sustained = self.history.iter().rev().take(3)
                    .all(|m| m.cpu_percent > self.threshold.cpu_percent);

                if sustained {
                    reports.push(BottleneckReport {
                        category: BottleneckCategory::Cpu,
                        severity,
                        current_value: current.cpu_percent,
                        threshold: self.threshold.cpu_percent,
                        suggestion: UpgradeSuggestion::EbpfOptimization {
                            program: "sched_monitor".to_string(),
                            description: "Install eBPF scheduler monitor to identify CPU-hungry processes".to_string(),
                        },
                        timestamp: current.timestamp,
                    });
                }
            }
        }

        // Memory bottleneck
        if current.memory_percent > self.threshold.memory_percent {
            let severity = if current.memory_percent > 95.0 {
                Severity::Critical
            } else {
                Severity::High
            };

            reports.push(BottleneckReport {
                category: BottleneckCategory::Memory,
                severity,
                current_value: current.memory_percent,
                threshold: self.threshold.memory_percent,
                suggestion: UpgradeSuggestion::HardwareUpgrade {
                    component: "RAM".to_string(),
                    specification: format!("Current usage: {:.1}%", current.memory_percent),
                },
                timestamp: current.timestamp,
            });
        }

        // IO bottleneck
        if current.disk_latency_us > self.threshold.disk_io_latency_us {
            reports.push(BottleneckReport {
                category: BottleneckCategory::Io,
                severity: Severity::High,
                current_value: current.disk_latency_us as f64,
                threshold: self.threshold.disk_io_latency_us as f64,
                suggestion: UpgradeSuggestion::KernelModule {
                    module: "io_scheduler".to_string(),
                    reason: format!("Disk latency {}us exceeds threshold {}us",
                        current.disk_latency_us, self.threshold.disk_io_latency_us),
                },
                timestamp: current.timestamp,
            });
        }

        reports
    }
}

#[async_trait]
impl PerceptionSource for BottleneckDetector {
    fn name(&self) -> &str {
        "bottleneck_detector"
    }

    async fn poll(&mut self) -> Result<Vec<PerceptionEvent>> {
        let metrics = self.collect_metrics()?;
        let bottlenecks = self.detect_bottlenecks(&metrics);

        // Store to history
        self.history.push_back(metrics);
        if self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }

        // Convert bottleneck reports to perception events
        let events: Vec<PerceptionEvent> = bottlenecks
            .into_iter()
            .map(|report| {
                let priority = match report.severity {
                    Severity::Critical => Priority::Critical,
                    Severity::High => Priority::High,
                    Severity::Medium => Priority::Normal,
                    Severity::Low => Priority::Low,
                };

                self.make_event(
                    EventData::System {
                        metric: format!("bottleneck_{:?}", report.category).to_lowercase(),
                        value: report.current_value,
                        unit: format!("suggestion: {:?}", report.suggestion),
                    },
                    priority,
                    EventCategory::System,
                )
            })
            .collect();

        if !events.is_empty() {
            info!("Detected {} bottlenecks", events.len());
        }

        Ok(events)
    }

    fn is_available(&self) -> bool {
        // Always available (reads /proc)
        true
    }
}

impl Default for BottleneckDetector {
    fn default() -> Self {
        Self::new(BottleneckThreshold::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bottleneck_threshold_default() {
        let t = BottleneckThreshold::default();
        assert_eq!(t.cpu_percent, 90.0);
        assert_eq!(t.memory_percent, 85.0);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::Medium < Severity::High);
        assert!(Severity::High < Severity::Critical);
    }

    #[tokio::test]
    async fn test_bottleneck_detector_poll() {
        let mut detector = BottleneckDetector::default();
        let events = detector.poll().await.unwrap();
        // May or may not detect bottlenecks depending on system state
        // Just verify it doesn't panic
        let _ = events;
    }

    #[tokio::test]
    async fn test_bottleneck_detector_is_available() {
        let detector = BottleneckDetector::default();
        assert!(detector.is_available());
    }

    #[test]
    fn test_bottleneck_report_serialization() {
        let report = BottleneckReport {
            category: BottleneckCategory::Cpu,
            severity: Severity::High,
            current_value: 95.0,
            threshold: 90.0,
            suggestion: UpgradeSuggestion::EbpfOptimization {
                program: "test".to_string(),
                description: "test".to_string(),
            },
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("Cpu"));
        assert!(json.contains("95"));
    }
}
