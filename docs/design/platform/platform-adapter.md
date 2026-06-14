# 平台适配器抽象 (PlatformAdapter)

> 跨平台通过 `PlatformAdapter` trait 实现，核心运行时仅依赖此接口，编译时通过 feature flag 选择平台实现。

**关联模块:** [IPC 与内核](kernel-ipc.md), [感知层](../perception/perception-layer.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| PlatformAdapter trait | ✅ Implemented | `platform/adapter.rs` | Trait with PlatformCapabilities, ServiceInfo, ServiceStatus |
| LinuxPlatformAdapter | ✅ Implemented | `platform/linux.rs` | systemd, /proc, /sys integration |
| AndroidPlatformAdapter | ✅ Implemented | `platform/android.rs` | Android platform adapter (stub) |
| BasicLinuxAdapter | ✅ Implemented | `platform/mod.rs` | Fallback Linux adapter |
| create_platform_adapter() | ✅ Implemented | `platform/mod.rs` | Factory function with feature-flag based selection |

---

## 1. 概述

OS-Agent 需要运行在 Linux PC、Android 和嵌入式开发板上。核心运行时与平台无关，通过 `PlatformAdapter` trait 抽象所有平台特定行为。编译时通过 feature flag 选择具体平台实现，运行时不可切换。

---

## 2. PlatformAdapter 接口

> **See [shared/traits.md](../shared/traits.md) for the canonical `PlatformAdapter` trait definition.**
> The table below provides platform-specific implementation notes for each method group.

| 方法 | 说明 | Linux 实现 | Android 实现 | 嵌入式实现 |
|------|------|-----------|-------------|-----------|
| `ipc_send/recv` | 进程间通信 | D-Bus / Unix socket | Binder | Serial/GPIO |
| `process_spawn/kill` | 进程生命周期 | systemd / fork | NDK / Intent | RTOS hooks |
| `fs_read/write/watch` | 文件系统访问 | /proc /sys / FUSE | AOSP APIs | SPIFFS/LittleFS |
| `permission_check/elevate` | 权限管理 | polkit / sudo | Root/ADB | 固定权限 |

---

## 3. 跨平台架构

```
                    ┌─────────────────────────────────┐
                    │      OS-Agent Core Runtime       │
                    │                                 │
                    │  ┌───────────┐  ┌────────────┐  │
                    │  │ 认知引擎  │  │ 记忆系统    │  │
                    │  │ Planner   │  │ Memory     │  │
                    │  │ Reasoner  │  │ 3-Layer    │  │
                    │  └───────────┘  └────────────┘  │
                    │  ┌───────────┐  ┌────────────┐  │
                    │  │ 编排引擎  │  │ 安全引擎    │  │
                    │  │ Orchestr. │  │ Policy     │  │
                    │  │ Selector  │  │ Sandbox    │  │
                    │  └───────────┘  └────────────┘  │
                    └────────────┬────────────────────┘
                                 │
                    ┌────────────┼────────────────────┐
                    │            │                     │
            ┌───────┴──────┐ ┌──┴──────────┐ ┌───────┴──────┐
            │   Linux      │ │  Android    │ │  嵌入式      │
            │   Adapter    │ │  Adapter    │ │  Adapter     │
            ├──────────────┤ ├─────────────┤ ├──────────────┤
            │ eBPF         │ │ Binder      │ │ GPIO         │
            │ systemd      │ │ AOSP APIs   │ │ I2C/SPI      │
            │ D-Bus        │ │ Accessibility│ │ UART         │
            │ /proc /sys   │ │ Root/ADB    │ │ RTOS hooks   │
            │ FUSE         │ │ NDK         │ │ NPU          │
            │ iptables     │ │ Intent      │ │ 传感器       │
            └──────────────┘ └─────────────┘ └──────────────┘
```

---

## 4. 设计原则

- **编译时绑定**: 核心运行时编译时通过 feature flag 选择平台实现，运行时不可切换
- **最小接口**: PlatformAdapter 只暴露核心能力，不包含平台特有功能
- **渐进实现**: Linux Adapter 优先实现，Android 和嵌入式按需扩展
- **降级兼容**: 低级平台（嵌入式）可以不实现某些方法（返回 `Unsupported`），核心功能保持可用

---

## 5. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| **原始设计文档** (`00-master.md` §2.4) | PlatformAdapter 抽象定义、跨平台架构图、方法对照表 |
| **设计总纲** (`00-master.md` §2.3) | 跨平台架构图（Linux/Android/嵌入式三层） |

---

## Implementation Summary

**Code location:** `crates/agent-core/src/platform/`

**Key types/traits implemented:**
- `PlatformAdapter` trait (`adapter.rs`) — cross-platform abstraction with send/recv, process spawn/kill, fs read/write/watch, permission check/elevate
- `PlatformCapabilities` struct (`adapter.rs`) — platform capability flags
- `ServiceInfo`, `ServiceStatus` (`adapter.rs`) — service lifecycle types
- `LinuxPlatformAdapter` (`linux.rs`) — systemd, /proc, /sys integration with full implementation
- `AndroidPlatformAdapter` (`android.rs`) — Android platform adapter (stub implementation)
- `BasicLinuxAdapter` (`mod.rs`) — fallback Linux adapter
- `create_platform_adapter()` factory (`mod.rs`) — feature-flag based platform selection

**Test coverage:** Unit tests exist for LinuxPlatformAdapter (4 tests including async tests). No tests for AndroidPlatformAdapter.
