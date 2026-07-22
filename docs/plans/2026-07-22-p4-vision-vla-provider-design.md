# P4 视觉、VLA 与策略 Provider 设计

> 状态：已裁决；必须在 P3 通过后实施

## 1. 目标与边界

路线要求 Camera/perception observation、frame/evidence、VLA/PolicyProvider、低频 proposal 与高频
edge rollout 分离，以及策略版本/来源/置信度/回放证据（路线文档 `:736-742`）。Fabric 已有视觉
`Image` 与 grounding seam（`crates/fabric/src/types/grounding.rs:6-54`），Dasein 已有 perception
聚合器（`crates/dasein/src/impl/perception/aggregator.rs:39`），P4 应扩展这些边界，不新建第二套视觉总线。

## 2. 固定裁决

- 首版输入只支持 RGB artifact，不把图像字节写入 turn context；
- `FrameRef` 包含 URI、sha256、MIME、尺寸、source time、frame ID、camera ID；
- VLA 只输出低频 `SkillProposal`，不能输出 ROS topic、关节、力矩或连续 actuator command；
- proposal 必须引用已注册 SkillId、参数、expected outcome、confidence、policy provenance；
- Kernel admission、P3 RobotHarness 和 OutcomeVerifier 不可绕过；
- 高速 rollout 在机器人/Provider 侧完成，Aletheon 只接收 bounded progress/result/evidence；
- 外部 PolicyProvider 复用通用 typed gRPC 治理：deadline、capabilities、health、大小限制、mTLS；
- 首个 E2E 固定为“RGB 观察目标→提出已注册技能→执行→P3 验证”，不做开放词汇任意控制。

## 3. 数据流

```text
Camera -> Bridge artifact store -> FrameRef/PerceptionObservation
       -> Dasein aggregation -> RobotHarness Observe
       -> PolicyProvider.propose(goal, frames, skills)
       -> ProposalValidator -> Kernel admission
       -> P2 skill execution -> P3 OutcomeVerifier -> Episode
```

## 4. 安全与验收

无 hash、过期 frame、超大图像、不受信 URI、未知 skill、低置信度、策略版本缺失全部拒绝。验收必须
证明图像不会进入日志/数据库正文，proposal 无法绕过白名单，策略成功但状态未变化仍由 P3 判失败。

## 5. 非目标

不训练模型、不运行高频控制、不保存视频流、不开放任意 prompt-to-actuator、不进行真机部署。
