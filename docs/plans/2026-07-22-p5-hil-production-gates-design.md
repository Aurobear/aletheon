# P5 HIL 与真机生产门禁设计

> 状态：已裁决；必须在 P2/P3 通过后实施，P4 不是 HIL 的硬前置

## 1. 目标

路线要求 HIL、网络故障注入、emergency stop、权限/安全审计、真机 allowlist，以及默认 simulation、
production 显式配置（路线文档 `:744-751`）。当前 Hardware 已有 `DeviceNamespace::Simulation`
（`crates/hardware/src/device.rs:24-33`）和 `SafetyState::SafeStopped`
（`crates/hardware/src/safety.rs:8`），但不存在通过审计的 Production namespace 启动门禁或独立 E-stop。

## 2. 固定裁决

- 分三层：SIL(P2) → HIL → Production；禁止从 SIL 直接跳真机；
- production 编译存在但运行默认关闭，必须 typed config 显式声明设备、endpoint、证书、allowlist、
  限值和已签名 gate evidence；
- Simulation/HIL/Production 使用不同 namespace、凭据、服务身份和审计目录；
- EmergencyStop 是独立高优先级本地路径，不等同于 Cancel/SafeStop；
- E-stop 触发后保持 latched，必须由本地人工复位，远程模型/RPC 无权复位；
- 真机首版只允许 stance、stop、safe stop 和一个人工审核低风险动作；
- 仿真运动限值不能继承到真机；真机值必须更严格并通过 HIL 测量；
- HIL 必测 latency/jitter/loss/duplicate/reorder/disconnect/lease/stale state；
- 任一 gate 证据缺失、过期、签名不符或设备不符，Production Provider 启动失败。

## 3. Gate 顺序

```text
Config preflight
 -> identity/certificate
 -> device manifest + namespace
 -> skill allowlist + limits
 -> E-stop channel self-test
 -> HIL evidence verification
 -> operator arming
 -> production provider ready
```

## 4. 安全职责

Aletheon 管授权、lease、审计与停止请求；Bridge 管失联 watchdog、本地零指令和 ROS 映射；机器人
控制器/安全 PLC/物理急停拥有最终制动权。软件不得宣称替代物理急停。

## 5. 验收

必须证明：默认配置不能启动真机；仿真凭据不能访问真机；每种网络故障都在规定 stop latency 内进入
安全状态；E-stop 不依赖 gRPC；远程 reset 被拒绝；审计链可关联 goal→operation→permit→lease→设备
回执→验证→停止；未签名/过期 HIL 报告不能解锁 Production。

## 6. 非目标

不定义硬件电气安全等级，不替代厂商安全认证，不自动部署未知机器人，不开放高风险动作。
