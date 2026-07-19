# Hardware Vertical Slice Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在单一 `hardware` crate 内，用 deterministic simulator 证明 Permit、lease、deadline、sequence、fail-safe、stop 与 receipt 的完整安全闭环，不连接真实 actuator。

**Architecture:** Kernel 授权 Operation capability，Hardware 验证设备领域约束，provider/simulator 执行，Executive 验证 receipt 并 settlement。Simulator 使用注入的 monotonic clock，所有拒绝也是结构化 receipt。

**Tech Stack:** Rust、Serde、现有 Kernel capability/permit contracts、deterministic simulator。

---

## 需求锚点

- 当前 simulator 只判断 `lease.is_some()`：`docs/arch/aletheon-hardware-control-platform-plan.md:20-34`，代码为 `crates/hardware/src/simulator.rs:47-56`。
- 第一纵向切片：`docs/arch/aletheon-hardware-control-platform-plan.md:66-76`。
- 安全不变量：`docs/arch/aletheon-hardware-control-platform-plan.md:89-97`。

### Task 1: 拆分 Hardware 内部领域模块

**Files:**
- Create: `crates/hardware/src/device.rs`
- Create: `crates/hardware/src/command.rs`
- Create: `crates/hardware/src/lease.rs`
- Create: `crates/hardware/src/telemetry.rs`
- Create: `crates/hardware/src/safety.rs`
- Create: `crates/hardware/src/provider.rs`
- Modify: `crates/hardware/src/lib.rs`

- [x] 类型拆入职责模块并从 `lib.rs` 精确 re-export；无新增 crate。
- [x] 定义 monotonic instant、command sequence、principal、operation 与 capability scope；控制 deadline 不使用 wall clock。
- [x] command 关联 operation/principal/sequence；lease 关联 operation/holder/device/scope/monotonic expiry。
- [x] `bash scripts/cargo-agent.sh test -p hardware` 通过。

### Task 2: 定义拒绝原因与可验证 receipt

**Files:**
- Modify: `crates/hardware/src/command.rs`
- Modify: `crates/hardware/src/safety.rs`
- Test: `crates/hardware/src/command.rs`

- [x] 定义 permit/lease/deadline/sequence/schema/safety 的穷举拒绝原因。
- [x] receipt 关联 operation、principal、device、command、sequence、typed decision、前后 safety state 与 monotonic timestamp。
- [x] 使用 `CommandDecision` enum，消除 bool/reason 矛盾状态。
- [x] 添加 serde 与 decision 不变量测试。

### Task 3: 注入 deterministic monotonic clock

**Files:**
- Create: `crates/hardware/src/clock.rs`
- Modify: `crates/hardware/src/simulator.rs`
- Test: `crates/hardware/src/simulator.rs`

- [x] 定义只读 monotonic clock 与 ManualClock；控制代码不调用 SystemTime。
- [x] Simulator 注入 clock，lease/deadline/receipt/telemetry 使用同一来源。
- [x] 覆盖接受、expiry 边界与时钟不可倒退。

### Task 4: 实现 lease 与 sequence 验证

**Files:**
- Modify: `crates/hardware/src/simulator.rs`
- Test: `crates/hardware/src/simulator.rs`

- [x] 验证 permit、holder、device、scope、expiry、schema、deadline 与 strictly increasing sequence。
- [x] 只有 accepted/fail-safe stop 推进 sequence；普通拒绝不改变位置，安全违规可进入 fail-safe。
- [x] 覆盖 owner/device/scope、expiry/deadline 与 duplicate/out-of-order。

### Task 5: fail-safe 与 stop 优先级

**Files:**
- Modify: `crates/hardware/src/safety.rs`
- Modify: `crates/hardware/src/simulator.rs`
- Test: `crates/hardware/src/simulator.rs`

- [x] 状态机包含 Ready/Active/Stopping/SafeStopped/Faulted。
- [x] disconnect、lease expiry 与 deadline violation 清零危险位置并进入 SafeStopped。
- [x] stop 高于普通命令，有效 Permit 下可绕过失效普通 lease，重复 stop 幂等。
- [x] 覆盖 disconnect、expiry、stop escalation、重复 stop 与 fault 拒绝。

### Task 6: Provider contract 与 telemetry evidence

**Files:**
- Modify: `crates/hardware/src/provider.rs`
- Modify: `crates/hardware/src/telemetry.rs`
- Modify: `crates/hardware/src/simulator.rs`

- [x] Provider 只接收私有构造的 ValidatedCommand，不授予 Permit/lease。
- [x] telemetry sequence 单调递增并关联 device、可选 operation 与 monotonic source time。
- [x] 原始 telemetry 留在 Hardware/provider；未接 Agora。

### Task 7: Kernel Permit 纵向集成

**Files:**
- Create: `crates/executive/tests/hardware_simulation.rs`
- Modify: `crates/executive/Cargo.toml`（仅增加现有 workspace `hardware` dev dependency，如确有需要）

- [x] 使用真实 Kernel production admission permit 构造 lease -> navigate -> stop -> verify receipt 流程；telemetry observe 由 Hardware 单测覆盖。
- [x] 覆盖无 permit、scope/operation mismatch、revoked、expiry 和 receipt operation mismatch。
- [x] 未连接 ROS/CAN/Serial/GPIO，未创建 provider crate。
- [x] Hardware 单测与 Executive `hardware_simulation` 定向测试通过。
- [x] **裁决:** 用户已授权自主继续；只接 simulator 测试 caller，不接真实 actuator。

### Task 8: 故障矩阵与实验状态

**Files:**
- Update: `architecture-status.toml`
- Modify: `docs/arch/aletheon-hardware-control-platform-plan.md`

- [x] deterministic tests 覆盖 expiry、disconnect、deadline、replay、wrong owner、stop、fault。
- [x] 真实 actuator 仍禁止；账本标记 `experimental_wired`，不标 production。
- [x] architecture check（28 findings、38 dependencies、4 paths，无新增）与 `git diff --check` 通过。

## 完成条件

- [x] 普通控制命令同时需要有效 Kernel Permit 与 ControlLease；安全 stop 需要有效 Permit 并可降级绕过失效 lease。
- [x] deadline 使用 monotonic clock，sequence 防重放/乱序。
- [x] disconnect/expiry/deadline 进入 fail-safe，stop 优先且幂等。
- [x] receipt 可关联 principal、Operation、device 和 command。
- [x] 仍只有一个 `hardware` crate，未连接真实 actuator。
