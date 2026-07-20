# Aletheon Hardware Control 收敛计划

> **Status:** Experimental simulator vertical slice implemented; no real actuator
>
> **Verified:** 2026-07-20

## 1. 定位

`hardware` 管理物理设备领域语义：设备身份、manifest、命令 schema、控制租约、
deadline、序列、遥测、停止与安全 receipt。

它不管理：

- Host 普通文件/进程/PTY（Platform）；
- Agent/Goal 调度（Executive）；
- 权限机制（Kernel）；
- 通用工具注册（Corpus）；
- 1 kHz 硬实时控制、状态估计或最终设备安全（Robot Edge Runtime）。

## 2. 当前代码现实

`crates/hardware/src/lib.rs` 只做精确模块导出；device、command、lease、clock、
telemetry、safety、provider 与 simulator 均留在单一 crate。Simulator 现在验证
Kernel permit 投影、lease holder/device/operation/scope/expiry、command schema、
monotonic deadline、strict sequence 与 safety state，并为接受和拒绝生成关联 receipt。

`crates/executive/tests/hardware_simulation.rs` 使用真实 Kernel admission permit 完成
navigate -> stop 的 deterministic 纵向测试，并覆盖缺失、scope/operation 不匹配、
revoked、expired 与 receipt operation mismatch。它仍是测试 caller，因此状态是
`experimental_wired`，不是 production。

## 3. 目标纵向链

```text
Executive Operation
    -> Kernel Capability Permit
    -> Hardware command validation
         identity / schema / lease / deadline / sequence / safety state
    -> simulator or provider
    -> command receipt + telemetry evidence
    -> Executive verification and settlement
```

任何 provider 都不能绕过 Kernel Permit 或设备本地 Safety Supervisor。

## 4. 单 crate 结构

```text
hardware/
  device
  command
  lease
  telemetry
  safety
  provider
  simulator
```

先以内部分模块表达边界。不得预建 `hardware-api`、`hardware-broker`、
`hardware-sim`、ROS、CAN、Serial、GPIO 或 vendor crate。

## 5. 第一纵向切片

1. 定义明确的 monotonic deadline 与 sequence；
2. lease 绑定 principal、device、capability scope 和 expiry；
3. simulator 使用可控时钟并拒绝 expired/wrong-owner/replayed command；
4. disconnect、deadline、lease expiry 触发预定义 fail-safe；
5. stop 命令优先级高于普通命令且幂等；
6. 每次接受/拒绝产生关联 Operation 的 receipt；
7. 通过 Kernel Capability 跑一次完整 observe -> lease -> command -> stop 测试。

完成前 Hardware 不接真实 actuator。

## 6. Provider 拆分门禁

Provider 默认是 `hardware` 内部模块。只有满足至少一项且存在真实 caller 时才独立：

- ROS 构建环境显著污染核心 workspace；
- CAN/Serial/vendor SDK 需要独立系统依赖或许可证；
- provider 是独立部署的 edge process；
- 原生 CI/HIL 需要独立发布周期。

拆分的是 adapter 隔离，不是重新建立 `hardware-api + broker` 层级。

## 7. 安全不变量

- 普通控制命令无有效 Permit 与 ControlLease 不执行；stop 仍要求有效 Permit，
  但可绕过已失效的普通 lease 进入更安全状态；
- deadline 使用 monotonic time；
- sequence 防止重放和乱序；
- 网络断开不保持最后危险命令；
- Aletheon stop 不取代设备物理急停；
- 原始高频 telemetry 不写入 Agora；
- 仿真、实验室、生产 namespace 不可隐式切换。

## 8. 验收

- deterministic simulator fault tests；
- lease expiry/disconnect/deadline/replay tests；
- stop escalation 与幂等测试；
- Kernel Permit 纵向集成测试；
- receipt 可关联 principal/Operation/device/command；
- 没有生产 caller 时保持 experimental；
- 未满足 driver 隔离门禁时不增加新 crate。
