# Architecture Convergence Master Plan

> **For agentic workers:** This file is the execution index. Implement the linked plans task-by-task and stop at every explicit confirmation point.

**Goal:** 让 Aletheon 在不继续拆 crate 的前提下，形成真实 Agent、Runtime、Platform 与 Hardware 闭环。

**Architecture:** Executive 是唯一 composition/verification/settlement authority；领域 crate 内聚自己的生命周期；Platform 与 Hardware 各保持单一 crate；所有“完成”必须由 evidence 和独立验收证明。

**Tech Stack:** Rust workspace、现有 architecture checks、`scripts/cargo-agent.sh`。

---

## 实施计划

1. [Platform Backend Convergence](./2026-07-19-platform-backend-convergence.md)
   - 接通真实 selector/probe。
   - 建立三 OS contract suite。
   - 迁移 Corpus 重复 Host 实现。

2. [Architecture Boundary Convergence](./2026-07-19-architecture-boundary-convergence.md)
   - MCP 配置归 Corpus。
   - 裁决 credential owner。
   - 缩窄 `execd -> corpus`。
   - 删除或接入三个实验抽象。

3. [Agent Production Closure](./2026-07-19-agent-production-closure.md)
   - 真实 AgentResult、coding evidence 与独立验收 receipt。
   - evidence-based verifier。
   - Runtime selector 与 Executive verification。
   - 三个真实 coding fixtures。
   - restart/cancel/timeout/orphan/false-success 门禁。

4. [Hardware Vertical Slice](./2026-07-19-hardware-vertical-slice.md)
   - Kernel Permit + ControlLease。
   - monotonic deadline + sequence。
   - fail-safe + idempotent stop。
   - deterministic simulator receipt 纵向闭环。

## 顺序与并行约束

```text
Platform Task 1-4 -----> Platform Corpus migration

MCP owner -------------> Agent production MCP caller migration

AgentResult/coding evidence --> independent verifier --> 3 coding fixtures

Hardware domain model --> simulator safety ---------> Kernel integration
```

- 不并发运行 Executive/workspace build。
- Hardware 可以在 Platform/Agent 计划之外独立推进，但接 Executive 前必须确认 API。
- Execd patch owner 同时影响 Platform 与 boundary 计划，只实施一次，以用户裁决为准。
- 每个计划中的“确认点”都是硬门禁；没有确认不得进入下一组件。

## 全局完成条件

- [x] 三个真实 coding fixtures 通过真实 Executive，并生成可重放 receipt。
- [x] AgentResult/coding report/harness receipt 的 output/usage/diff/evidence 真实，verification 不依赖最终文本或消息数量。
- [x] 当前 Linux Platform backend 真实接通；Windows/macOS 保持 unverified，只在原生 runner 通过后标记完成。
- [x] Hardware simulator 通过 Permit/lease/deadline/replay/fail-safe/stop 纵向测试。
- [x] MCP 与 credential 各有唯一明确 owner。
- [x] 所有保留抽象都有 production caller；无 caller 的 Runtime lifecycle、Corpus Host 树和 ContainerBackend 已删除。
- [x] Workspace 保持 16 个领域/入口 crate 和 2 个 example package，不出现带连字符或 `api/types/common/broker` 拆分 crate。
- [x] `bash scripts/architecture-check.sh` 与 `git diff --check` 通过（2026-07-20）。
