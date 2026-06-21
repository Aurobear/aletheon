# Linux System Integration

Aletheon is a system-level agent, not an application-level one. It integrates with the Linux kernel and system services through four mechanisms: eBPF, systemd, FUSE, and /proc+/sys. This document explains each integration point, the current implementation status, and the security model.

---

## Integration Layers

```
+-------------------------------------------------------------+
|                    Aletheon Agent                             |
+-------------------------------------------------------------+
|  Application Layer                                            |
|  D-Bus (IPC) / systemd (service management) / FUSE (VFS)    |
+-------------------------------------------------------------+
|  Kernel Layer                                                 |
|  eBPF (event monitoring) / procfs / sysfs / journald         |
+-------------------------------------------------------------+
|  Hardware Layer                                               |
|  Sensors / Device control / Resource monitoring              |
+-------------------------------------------------------------+
```

Design principle: **use the kernel, do not modify it.** Aletheon leverages existing Linux interfaces (eBPF, procfs, journald) without requiring custom kernel modules.

---

## eBPF Integration

### What eBPF Does for Aletheon

eBPF allows Aletheon to attach lightweight programs to kernel tracepoints and collect events without modifying the kernel. The agent uses this for:

- **Syscall monitoring:** Track which system calls processes make
- **Network monitoring:** Observe connections and traffic patterns
- **File system monitoring:** Watch file access and modification events
- **Performance profiling:** Collect CPU, memory, and I/O metrics at kernel level

### Current Implementation

The eBPF perception source exists in `crates/dasein/src/impl/perception/sources/ebpf_source.rs`.

| Feature | Status | Notes |
|---------|--------|-------|
| Mock /proc fallback | Implemented | Works on all Linux versions |
| Real eBPF ring buffer reading | Not implemented | Planned for Phase 5 |
| BPF program loading | Not implemented | Requires root + kernel 5.8+ |

The current implementation falls back to reading `/proc` and `/sys` directly, which provides the same data without kernel-version dependencies. The mock source is fully functional for development and testing.

### How Perception Events Flow

```
/proc, journald, inotify --> PerceptionManager (5s poll interval)
  --> EventAggregator (dedup, rate limit, priority boost)
  --> mpsc channel
  --> PerceptionBridge
    --> Critical/High: immediate injection as system message
    --> Medium/Low: buffered, flushed every 30s
  --> Engine.drain_perceptions() --> injected into message history
```

The perception system is the bridge between kernel-level events and the agent's cognitive loop. Critical events (OOM killer, disk full) are injected immediately. Low-priority events (routine metrics) are batched.

See [Perception Layer](../design/body/perception.md) for the full design.

---

## systemd Integration

### Service Management

Aletheon runs as a systemd service, gaining automatic restart, dependency management, and journal logging.

**Unit file:**

```ini
# /etc/systemd/system/aletheon.service
[Unit]
Description=Aletheon Agent Runtime
After=network.target

[Service]
Type=simple
User=aletheon
ExecStart=/usr/bin/daemon
Restart=always
RestartSec=5
# Security hardening
ProtectSystem=strict
ProtectHome=read-only
PrivateTmp=yes
NoNewPrivileges=yes
ReadWritePaths=/var/lib/aletheon /tmp/aletheon.sock

[Install]
WantedBy=multi-user.target
```

**Operations:**

```bash
sudo systemctl start aletheon
sudo systemctl status aletheon
sudo journalctl -u aletheon -f
```

### Platform Adapter

Aletheon abstracts platform-specific operations behind a `PlatformAdapter` trait:

```rust
pub trait PlatformAdapter {
    fn install_service(&self, config: &ServiceConfig) -> Result<()>;
    fn remove_service(&self, name: &str) -> Result<()>;
    fn start_service(&self, name: &str) -> Result<()>;
    fn stop_service(&self, name: &str) -> Result<()>;
    fn service_status(&self, name: &str) -> Result<ServiceStatus>;
}
```

The Linux implementation uses `systemctl` and D-Bus. An Android implementation exists as a stub.

See [Platform Adapter](../design/body/platform.md) for details.

---

## FUSE Virtual Filesystem

### What FUSE Provides

Aletheon mounts a virtual filesystem that exposes agent state, perception data, and control interfaces through standard Unix file operations. This means any shell script or system tool can interact with the agent using `cat`, `echo`, and `tail -f`.

**Mount structure:**

```
/mnt/agent/
  context/            # Agent context (read-only)
    current_task.md
    memory_summary.md
  controls/           # Agent controls (read-write)
    pause             # Write "1" to pause, "0" to resume
    config.toml       # Live config (allowlisted keys only)
  sensors/            # Perception data (read-only)
    system/
      loadavg
      meminfo
      diskstats
      net_dev
  logs/               # Agent logs (read-only, tail -f friendly)
    agent.log
    tool_calls.log
  agents/             # Multi-agent status (read-only)
    <agent-name>/status.json
```

### Implementation Details

The FUSE filesystem is implemented behind a `fuse` feature flag in `crates/dasein/src/impl/perception/fuse/`:

| Component | File | Purpose |
|-----------|------|---------|
| `AgentFs` | `filesystem.rs` | In-memory virtual filesystem with read/write/readdir |
| `FuseMount` | `mount.rs` | `fuse3::path::PathFileSystem` integration |
| `StateProvider` | `provider.rs` | Abstraction for data sources |
| `LiveStateProvider` | `provider.rs` | Reads live state from /proc |
| `ControlsValidator` | `controls.rs` | Write validation for /controls/ (allowlisted keys, TOML syntax check) |

When the `fuse` feature is disabled, `FuseMount` operates in stub mode (always reports unmounted). The in-memory `AgentFs` API works regardless of the feature flag.

### Usage

```bash
# Mount
aletheon-fuse /mnt/agent

# Read agent status
cat /mnt/agent/agents/default/status.json

# Pause the agent
echo "1" > /mnt/agent/controls/pause

# Watch logs in real time
tail -f /mnt/agent/logs/agent.log

# Read system sensors
cat /mnt/agent/sensors/system/loadavg
```

See [FUSE Interface](../design/body/fuse.md) for the full design.

---

## /proc and /sys Integration

Aletheon reads system state directly from procfs and sysfs -- the same interfaces that `top`, `free`, and `df` use.

**Key data sources:**

| Path | Data | Usage |
|------|------|-------|
| `/proc/loadavg` | System load averages | ResourceGovernor throttling |
| `/proc/meminfo` | Memory usage | OOM prevention, context budget |
| `/proc/stat` | CPU usage | Performance profiling |
| `/proc/<pid>/status` | Per-process info | Process monitoring |
| `/proc/net/dev` | Network interface stats | Network monitoring |
| `/proc/diskstats` | Disk I/O stats | I/O bottleneck detection |
| `/sys/class/thermal/` | Temperature sensors | Hardware health monitoring |

The perception system polls these paths at a configurable interval (default 5 seconds) and feeds events into the cognitive loop.

---

## D-Bus Integration

D-Bus provides IPC between Aletheon and other system services.

```rust
#[dbus_interface(name = "org.aletheon.Agent")]
impl AgentInterface {
    fn execute_task(&self, task: &str) -> Result<String, Error>;
    fn get_status(&self) -> Result<AgentStatus, Error>;
}
```

D-Bus is used primarily by the `PlatformAdapter` for systemd interaction and could be exposed for third-party integration.

---

## Security Model

System-level access requires careful security controls.

### Permission Hierarchy

| Level | Access | Examples |
|-------|--------|----------|
| L0 | Read-only | Read /proc, /sys, logs |
| L1 | Bounded write | Write to agent workspace, FUSE controls |
| L2 | Privileged | Start/stop services, modify system config |
| L3 | Destructive | Kernel operations, filesystem formatting |

Most operations run at L0-L1. L2-L3 require explicit user approval.

### Sandboxing

Tools execute in sandboxes that restrict system access:

- **bubblewrap:** Full namespace isolation, seccomp filters, cgroup limits
- **process:** Basic process isolation (UID/GID separation)
- **noop:** No isolation (development only)

### Resource Governance

The `ResourceGovernor` enforces limits on:
- CPU time per tool execution
- Memory usage per sandbox
- File descriptor count
- Network connections

An `EmergencyKillswitch` can halt all agent activity on multi-trigger conditions (repeated failures, resource exhaustion, integrity violation).

See [Security Model](../design/body/security.md) for the full design.

---

## Best Practices

1. **Least privilege:** Only request the permissions you need. Run as a dedicated `aletheon` user, not root.
2. **Audit logging:** All system access is logged to the audit trail. Review it.
3. **Sandbox everything:** Tool execution should use bubblewrap or process isolation by default.
4. **Resource limits:** Set cgroup limits to prevent resource exhaustion.
5. **Feature flags:** eBPF and FUSE are behind feature flags. Enable only what your deployment needs.

---

## Related Documents

- [Perception Layer](../design/body/perception.md) -- event sources and processing pipeline
- [FUSE Interface](../design/body/fuse.md) -- virtual filesystem design
- [Security Model](../design/body/security.md) -- policy engine and sandboxing
- [Platform Adapter](../design/body/platform.md) -- systemd and Android abstraction
- [Architecture Overview](../design/architecture-overview.md) -- full system architecture
- [Linux Kernel Patterns](../plans/2026-06-14-linux-kernel-patterns.md) -- eBPF, FUSE, io_uring integration patterns
