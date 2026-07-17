# Platform Driver & Hardware Control — 多平台适配与实际硬件控制

> **Status:** Design / Proposed（未实现；实现需另行批准）
>
> **Date:** 2026-07-17
>
> **Baseline:** `dev` HEAD
>
> **一句话:** 现有 "platform driver" 层是**桌面/OS 自动化 HAL**（屏幕/输入/剪贴板/无障碍/OCR/服务管理），**不是**机器人硬件控制层。要"控制实际硬件"，需要在现有 HAL 之上新建一条 effector/fieldbus 驱动栈 + 实时控制回路，并以 `BodyRuntime::execute(Action)` 为集成缝。

---

## 1. 现状盘点（代码级）

### 1.1 现有驱动都在 `crates/corpus/src/drivers/`（模块树 `drivers/mod.rs:1`）

**已实现（真实后端，Linux）**
| 能力 | 位置 |
|------|------|
| X11 display | `display/x11.rs:20` |
| X11 window (EWMH) | `display/window_x11.rs:32` |
| DRM/framebuffer | `display/drm.rs:47` |
| Clipboard (X11) | `display/clipboard_x11.rs:42` |
| uinput 输入 | `input/uinput.rs:351` |
| Tesseract OCR | `ocr/tesseract.rs:29` |
| AT-SPI 无障碍 | `a11y/atspi.rs:82` |
| Linux 平台 (D-Bus) | `platform/linux.rs:33` |
| 启动监控 | `platform/boot.rs:1`（830 行） |
| Sandbox 原语 (seccomp/cgroups/ns) | `sandbox_driver/mod.rs:1` |

**Stub / 未实现**
| 项 | 位置 | 说明 |
|----|------|------|
| Android | `platform/android.rs:3,25` | 头注为 stub（缺 Binder/NDK），但 **shell 回退已实现**（dumpsys/getprop/setprop `android.rs:70-216`）——只缺原生 Binder IPC |
| proc 驱动 | `proc/mod.rs:1` | 单行 `//! TODO: Phase 7/8` |
| io 驱动 | `io/mod.rs:1` | 单行 `//! TODO: Phase 7/8` — **未来设备绑定的天然落点** |

> 注：审计 `06` 声称 "10/13 完整"，与其自身表格（12 行）不自洽；以本盘点为准。

### 1.2 抽象方式：一能力一 trait（无统一 `Driver` trait），均 `Send + Sync`
- `PlatformAdapter` `platform/adapter.rs:39`（服务管理/主机信息/提权/能力查询）
- `DisplayDriver` `display/mod.rs:5`、`InputDriver` `input/mod.rs:5`、`A11yDriver` `a11y/mod.rs:7`、`OcrDriver` `ocr/mod.rs:8`、`ClipboardDriver` `display/clipboard.rs:3`、`WindowManager` `display/window.rs:16`
- **高层 effector HAL：** `BodyRuntime` `crates/fabric/src/include/body.rs:73` — `execute(Action)/check/capabilities`；其 doc-comment（`body.rs:71`）把 "shell, filesystem, browser, ROS" 列为示例后端，**但无任何 ROS 后端实现它**。

### 1.3 运行时选择/注册
- 平台适配：`create_platform_adapter()` `platform/mod.rs:17` — 先探测 Android（`is_android()` 查 `/system/build.prop`），再 `#[cfg(feature="dbus")]` Linux，否则 `BasicLinuxAdapter` 回退（`platform/mod.rs:31-35`）。
- 能力驱动：`DriverFactory` `drivers/factory.rs:5` — `try_input/try_display/...`，由 `#[cfg(feature=…)]` + `#[cfg(target_os="linux")]` + 运行时探测（`/dev/uinput`、`$DISPLAY`、`$DBUS_SESSION_BUS_ADDRESS`）门控；不可用时返回 `None` **静默降级**。

### 1.4 实际硬件控制面：**当前为零**
仓库级 grep `ethercat|PDO|GPIO|actuator|servo|rclrs|ros2|modbus|serialport|canbus|torque|joint|1000Hz|control loop`：**无真实命中**（仅 MCP 查询串 "servo"、TUI 注释 "like ros2 topic hz"、`body.rs:71` 的 doc "ROS" 提及）。无实时回路、无现场总线、无传感/执行器 I/O、无 ROS 桥。`drivers/types.rs` 只建模 UI 原语（MouseButton/Key/Element/UiTree/OcrResult）。

---

## 2. 结论：两个不同的"多平台"

用户诉求含两层，必须分开：

| 维度 | 含义 | 现状 |
|------|------|------|
| **A. OS 多平台适配** | 让现有桌面 HAL 跑在 Linux 之外（Android 原生、macOS、Windows、headless server） | Linux 完整；Android 半（shell 回退在，Binder 缺）；其余无 |
| **B. 实际硬件控制** | 控制执行器/传感器/现场总线（机器人本体） | **完全缺失，需从零构建** |

本计划两条线并行但独立，B 是主诉求（"为了控制实际硬件"）。

---

## 3. B 线设计：实际硬件控制栈（主）

### 3.1 集成缝：`BodyRuntime`
现有 `BodyRuntime::execute(Action)`（`fabric/src/include/body.rs:73`）已是"意图→效应"的高层 HAL 契约，doc-comment 甚至预留了 ROS。**以它为唯一集成点**：新增的硬件后端实现 `BodyRuntime`（或其下沉的 effector trait），上层（工具/意识层）无需知道底层是 X11 还是伺服电机。

### 3.2 新增分层（建议放 `crates/corpus/src/drivers/hardware/`，或独立 `crates/embodiment/`）

```text
  上层  Action / 工具调用 / 意识层动作提案
          │
   ┌──────▼───────────────────────────── BodyRuntime (fabric trait, 已存在) ───────┐
   │  EffectorDriver (新 trait)            SensorDriver (新 trait)                   │
   │   - command(JointCmd/IoCmd)            - read(SensorFrame)                       │
   │   - capabilities() -> HardwareCaps     - subscribe(stream)                       │
   └──────┬──────────────────┬──────────────────┬──────────────────┬───────────────┘
          │                  │                  │                  │
     EtherCAT master     CAN/Modbus         serial/tty          GPIO/PWM
     (SOEM/ethercrab)    (socketcan)        (serialport)        (gpiod)
          │
   ┌──────▼─────────────────────────────────────────────────────┐
   │  RT Control Loop（确定性调度，100Hz–1kHz）                    │
   │   - 独立 OS 线程 + SCHED_FIFO / PREEMPT_RT                     │
   │   - 周期读传感 → 计算 → 写执行器（PDO 循环）                    │
   │   - 与 async 世界通过 lock-free ring / 双缓冲解耦（勿在 RT 内 .await）│
   └──────────────────────────────────────────────────────────────┘
```

### 3.3 关键设计决定（需评审）
1. **RT 边界纪律**：控制回路**不得**在 tokio async 上下文里跑，也不得在回路内做堆分配/加锁/`.await`。async 世界（daemon/turn pipeline）与 RT 回路之间用无锁 ring buffer + 命令/状态双缓冲通信。这是嵌入式实时铁律，必须在设计阶段定死。
2. **类型建模**：新增 `hardware/types.rs`（`JointState`、`JointCommand`、`SensorFrame`、`PdoMap`、`HardwareCaps`），与现有 UI 原语 `drivers/types.rs` 分离。
3. **落点复用**：`proc/mod.rs` 与 `io/mod.rs` 现为空 TODO stub——它们是低层设备绑定的天然归宿，无需新建模块树根。
4. **安全**：硬件动作必须经过与工具同级的治理（权限层 + 意识层软否决 R3），且额外需要**急停 (E-stop) / 看门狗 / 限位**——软件否决之外的硬安全，本计划要求"fail-safe 默认断电/回中"。
5. **仿真优先**：先提供一个 `MockHardware` / 仿真后端（对齐现有各 trait 都有 mock 的模式，如 `display/mod.rs:24`），让上层与 CI 在无真机时可测。

### 3.4 分阶段（B 线）
```text
B0  契约与仿真   定义 EffectorDriver/SensorDriver + hardware/types.rs + MockHardware；BodyRuntime 接一个仿真后端。零真机、可 CI。
B1  单总线打通   选一条总线（建议 serial 或 CAN，最易验证）实现一个真实后端，非实时、请求-响应级。
B2  RT 回路      引入确定性控制线程 + 无锁 ring；把 B1 后端接入周期回路，先低频（100Hz）。
B3  EtherCAT/PDO 现场总线 master + PDO 映射；提升到 1kHz；加看门狗/急停/限位。
B4  ROS 桥(可选) 若需与 ROS2 生态互操作，实现一个 BodyRuntime 的 ROS 后端（对齐 body.rs:71 的预留）。
```

### 3.5 验收（B 线，设计意图）
- AC-B0.1：`BodyRuntime` 接仿真后端后，一个 `Action` 能驱动 `MockHardware` 并回读 `SensorFrame`；CI 无真机通过。
- AC-B2.1：RT 回路以设定频率运行，抖动（jitter）在目标阈值内；回路内零分配/零 `.await`（可用 `#![no_std]` 子模块或 lint/审查佐证）。
- AC-B3.1：看门狗超时 / 急停触发时，执行器进入 fail-safe（断电或回中），且该路径有测试。
- AC-B*.2：硬件动作全部经过权限治理；无治理绕过（对齐 CI 的 `Tool::execute` enforcement 思路）。

---

## 4. A 线设计：OS 多平台适配（次）

现有 `DriverFactory` + `create_platform_adapter` 的 `#[cfg]` + 运行时探测模式**已是正确骨架**，A 线是"沿骨架补后端"，风险低：

| 平台 | 现状 | 建议 |
|------|------|------|
| Linux | 完整 | 维持 |
| Android | shell 回退在，缺 Binder | 若需原生能力，补 Binder/NDK 后端（`android.rs`）；否则明确"shell 回退即支持边界" |
| macOS | 无 | 新增 `platform/macos.rs` + CoreGraphics/Accessibility 后端，接入 factory 的 cfg 分支 |
| Windows | 无 | 新增 `platform/windows.rs` + Win32/UIA 后端 |
| headless server | `BasicLinuxAdapter` 回退 | 明确无 display 时的降级契约（已 `None` 静默降级，建议改为可诊断的显式能力表） |

**A 线原则**：不改 trait，只加 `#[cfg(target_os=…)]` 后端 + factory 分支；`capabilities()` 必须诚实反映当前平台可用能力（供上层/诊断查询），避免"静默降级"变成"静默失败"。

---

## 5. 风险与不变量
1. **诚实的能力表**：任何平台/硬件后端的 `capabilities()` 必须真实；上层依据它决定能否做某动作，谎报会导致意识层/工具层误判。
2. **RT 与 async 严格解耦**：违反即引入不可预测延迟（参见 `dataflow` skill 关注的 100Hz↔1kHz 频率边界与帧丢失）。
3. **硬件 fail-safe 优先于一切软件逻辑**：急停/看门狗/限位是硬约束，不受意识层或工具层影响。
4. **仿真可测**：无真机也能在 CI 验证上层逻辑，避免硬件成为唯一验证路径。

**里程碑文件（已拆分）** —— 本文是总设计；下列为逐里程碑可执行详单（触及文件、任务分解、验收、依赖）：
- B0 → [`2026-07-17-platform-b0-contract-and-sim-detailed-plan.md`](./2026-07-17-platform-b0-contract-and-sim-detailed-plan.md)
- B1–B2 → [`2026-07-17-platform-b1-b2-bus-and-rt-loop-detailed-plan.md`](./2026-07-17-platform-b1-b2-bus-and-rt-loop-detailed-plan.md)
- B3(+B4) → [`2026-07-17-platform-b3-fieldbus-failsafe-detailed-plan.md`](./2026-07-17-platform-b3-fieldbus-failsafe-detailed-plan.md)
- A 线 → [`2026-07-17-platform-a-os-multiplatform-detailed-plan.md`](./2026-07-17-platform-a-os-multiplatform-detailed-plan.md)

---

## 6. 与其它计划的关系
| 计划 | 关系 |
|------|------|
| `2026-07-17-conscious-core-engineering-plan.md` | R3 软否决作用于 `Action`；硬件动作是 `Action` 的一种——意识层可**收紧**硬件动作，但**不得**替代硬件 fail-safe |
| `2026-07-17-tool-execution-hardening-plan.md` | 硬件动作应纳入同一治理/沙箱审查框架 |
| `2026-07-17-kernel-application-layer-separation-plan.md` | `EffectorDriver`/`SensorDriver` 契约应放 `fabric`（与 `BodyRuntime` 同层），后端放 corpus/embodiment，遵守 kernel/app 依赖方向 |

---

## 7. 完成定义（DoD）
- B 线：`BodyRuntime` 有仿真后端（B0）+ 至少一条真实总线（B1）+ 确定性 RT 回路（B2）+ fail-safe 路径有测试。
- A 线：目标平台的 `PlatformAdapter`/能力驱动有后端或明确的能力边界声明；`capabilities()` 诚实。
- 硬件与桌面动作共用同一治理路径，无绕过。
