use fabric::WallTime;
use serde::{Deserialize, Serialize};

/// Unique event identifier.
pub type EventId = u64;

/// Perception event from system monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptionEvent {
    pub id: EventId,
    pub timestamp: WallTime,
    pub source: EventSource,
    pub category: EventCategory,
    pub priority: Priority,
    pub data: EventData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventSource {
    Proc,
    Inotify,
    Journald,
    Ebpf,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventCategory {
    File,
    Process,
    Network,
    System,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventData {
    // File events
    FileCreated {
        path: String,
    },
    FileModified {
        path: String,
    },
    FileDeleted {
        path: String,
    },

    // Process events
    ProcessStarted {
        pid: u32,
        comm: String,
        cmdline: Option<String>,
    },
    ProcessExited {
        pid: u32,
        comm: String,
        exit_code: Option<i32>,
    },
    HighCpu {
        pid: u32,
        comm: String,
        cpu_percent: f64,
    },

    // System events
    MemoryPressure {
        available_mb: u64,
        total_mb: u64,
    },
    DiskPressure {
        mount: String,
        available_gb: f64,
        total_gb: f64,
    },
    LoadAvg {
        load1: f64,
        load5: f64,
        load15: f64,
    },

    // Service events
    ServiceStateChanged {
        name: String,
        old_state: String,
        new_state: String,
    },
    JournalEntry {
        unit: String,
        message: String,
        priority: u8,
    },

    // System metrics (from /proc, eBPF, etc.)
    System {
        metric: String,
        value: f64,
        unit: String,
    },

    // eBPF-sourced kernel events
    EbpfSched {
        prev_pid: u32,
        next_pid: u32,
        prev_comm: String,
        next_comm: String,
        prev_state: i64,
    },
    EbpfNet {
        pid: u32,
        comm: String,
        iface: String,
        bytes: u64,
        direction: String, // "tx" or "rx"
    },
    EbpfBlock {
        pid: u32,
        comm: String,
        dev: u32,
        sector: u64,
        bytes: u64,
        latency_ns: u64,
    },
    EbpfSyscall {
        pid: u32,
        comm: String,
        syscall_nr: i64,
        args: [u64; 6],
    },

    // Generic
    Raw {
        message: String,
    },
}

impl PerceptionEvent {
    pub fn is_critical(&self) -> bool {
        self.priority == Priority::Critical
    }

    /// Human-readable summary for injection into context.
    pub fn summary(&self) -> String {
        match &self.data {
            EventData::FileCreated { path } => format!("File created: {}", path),
            EventData::FileModified { path } => format!("File modified: {}", path),
            EventData::FileDeleted { path } => format!("File deleted: {}", path),
            EventData::ProcessStarted { comm, pid, .. } => {
                format!("Process started: {} (pid {})", comm, pid)
            }
            EventData::ProcessExited {
                comm,
                pid,
                exit_code,
            } => {
                format!(
                    "Process exited: {} (pid {}, exit {:?})",
                    comm, pid, exit_code
                )
            }
            EventData::HighCpu {
                comm, cpu_percent, ..
            } => {
                format!("High CPU: {} ({:.1}%)", comm, cpu_percent)
            }
            EventData::MemoryPressure {
                available_mb,
                total_mb,
            } => {
                format!(
                    "Memory pressure: {}/{} MB available",
                    available_mb, total_mb
                )
            }
            EventData::DiskPressure {
                mount,
                available_gb,
                total_gb,
            } => {
                format!(
                    "Disk pressure on {}: {:.1}/{:.1} GB available",
                    mount, available_gb, total_gb
                )
            }
            EventData::LoadAvg {
                load1,
                load5,
                load15,
            } => {
                format!("Load avg: {:.2} {:.2} {:.2}", load1, load5, load15)
            }
            EventData::ServiceStateChanged {
                name,
                old_state,
                new_state,
            } => {
                format!("Service {} changed: {} -> {}", name, old_state, new_state)
            }
            EventData::JournalEntry { unit, message, .. } => {
                format!("[{}] {}", unit, message)
            }
            EventData::System {
                metric,
                value,
                unit,
            } => {
                format!("System metric: {} = {} {}", metric, value, unit)
            }
            EventData::EbpfSched {
                prev_comm,
                next_comm,
                ..
            } => {
                format!("eBPF sched: {} -> {}", prev_comm, next_comm)
            }
            EventData::EbpfNet {
                comm,
                iface,
                bytes,
                direction,
                ..
            } => {
                format!(
                    "eBPF net: {} {} {} bytes on {}",
                    comm, direction, bytes, iface
                )
            }
            EventData::EbpfBlock {
                comm,
                bytes,
                latency_ns,
                ..
            } => {
                format!(
                    "eBPF block: {} {} bytes ({}ns latency)",
                    comm, bytes, latency_ns
                )
            }
            EventData::EbpfSyscall {
                comm, syscall_nr, ..
            } => {
                format!("eBPF syscall: {} nr={}", comm, syscall_nr)
            }
            EventData::Raw { message } => message.clone(),
        }
    }
}
