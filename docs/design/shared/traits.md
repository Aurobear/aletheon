# 共享 Trait (Shared Traits)

> 跨模块接口定义。

**This file is the CANONICAL definition. Other docs MUST reference this file, not redefine these traits.**

**关联模块:** 所有模块
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| LlmProvider | ✅ Implemented | `llm/provider.rs` | Provider trait with complete/complete_stream |
| Tool | ✅ Implemented | `tool/mod.rs` | Includes permission_level(), exposure(), concurrency_class() |
| PlatformAdapter | ✅ Implemented | `platform/adapter.rs`, `platform/linux.rs`, `platform/android.rs` | Linux (systemd/D-Bus) + Android (getprop/dumpsys) |
| MemoryStore | ⬜ Planned | — | Memory uses different API than this trait |

**Stale reference fixed:** Tool trait doc was missing `permission_level()`, `exposure()`, `concurrency_class()` -- code has these methods.

---

## 1. LLM Provider

**LlmProvider** — LLM 提供者接口，支持 complete（同步）和 complete_stream（流式）两种推理模式。
- 代码位置: `llm/provider.rs`
- 方法: complete, complete_stream, name, max_context_length

## 2. Tool

> **Canonical definition** — superset of all fields from tool-system, platform-adapter, and loop-detector docs.

**Tool** — 统一工具接口，包含 name, description, input_schema, permission_level (L0-L3), needs_sandbox, exposure (ToolExposure), concurrency_class (ConcurrencyClass), execute。
- 代码位置: `tool/mod.rs`
- `ToolExposure` 和 `ConcurrencyClass` 枚举定义在 [execution/tool-system.md](../execution/tool-system.md)

## 3. PlatformAdapter

> ✅ **Implemented** — Linux and Android adapters complete.

**PlatformAdapter** — 平台适配器接口，涵盖 IPC（send/recv）、进程生命周期（spawn/kill）、文件系统（read/write/watch）、权限（check/elevate）。
- 代码位置: `platform/adapter.rs` (trait), `platform/linux.rs` (Linux/D-Bus), `platform/android.rs` (Android stub)
- Platform-specific implementation notes are in [platform/platform-adapter.md](../platform/platform-adapter.md)

## 4. MemoryStore

> ⬜ **Planned** — 保持完整设计。

**MemoryStore** — 记忆存储接口，包含 read_core, write_core, search_recall, search_archival, record_outcome。
- 代码位置: 尚无实现（Memory 使用不同 API）

---

## Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| LlmProvider trait | `llm/provider.rs` | `LlmProvider`, `LlmRequest`, `LlmResponse` |
| Tool trait | `tool/mod.rs` | `Tool`, `ToolExposure`, `ConcurrencyClass` |
| PlatformAdapter trait | `platform/adapter.rs` | `PlatformAdapter` |
| Linux adapter | `platform/linux.rs` | `LinuxPlatformAdapter` (systemd/D-Bus) |
| Android adapter | `platform/android.rs` | `AndroidPlatformAdapter` (getprop/dumpsys) |
