# Platform A — OS 多平台适配（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-platform-driver-hardware-control-plan.md` §4（A 线）
> **批次:** A（次线，风险低；与 B 线独立并行）
> **目标:** 让现有桌面/OS 自动化 HAL 跑出 Linux 之外（Android 原生、macOS、Windows、headless），沿用现有 `#[cfg]` + 运行时探测骨架，不改 trait。

## 触及文件（锚点）
- `crates/corpus/src/drivers/factory.rs:5` — `DriverFactory`（`try_input/try_display/...`，`#[cfg]` + 运行时探测门控）
- `crates/corpus/src/drivers/platform/mod.rs:17` — `create_platform_adapter()`（Android→Linux(dbus)→BasicLinux 回退 `:31-35`）
- `crates/corpus/src/drivers/platform/android.rs:70-216` — shell 回退已实现，缺原生 Binder/NDK
- 新增 `platform/macos.rs`、`platform/windows.rs` — 各自后端 + factory cfg 分支
- `crates/corpus/src/drivers/platform/adapter.rs:39` — `PlatformAdapter` trait（不改）

## 任务分解（TDD）
1. **T1** macOS：`platform/macos.rs` + CoreGraphics/Accessibility 后端，接入 factory 的 `#[cfg(target_os="macos")]` 分支。
2. **T2** Windows：`platform/windows.rs` + Win32/UIA 后端，接入 cfg 分支。
3. **T3** Android：若需原生能力，补 Binder/NDK 后端；否则**明确声明**"shell 回退即支持边界"。
4. **T4** headless：无 display 时，把当前 `None` 静默降级改为**可诊断的显式能力表**。
5. **T5** 所有平台 `capabilities()` 诚实反映当前可用能力（供上层/诊断），避免"静默降级"变"静默失败"。

## 验收（来自父计划）
- 目标平台的 `PlatformAdapter`/能力驱动有后端或明确的能力边界声明；`capabilities()` 诚实。

## 不变量 / 风险
- **不改 trait**：只加 `#[cfg(target_os=…)]` 后端 + factory 分支。
- **诚实的能力表**：谎报会导致上层（工具层/意识层）误判。

## 依赖
- 无（与 B 线独立；可与硬件线并行推进）。
