# P3 RobotHarness、世界模型与结果验证设计

> 状态：已裁决，可进入实施
>
> 前置：P2 MuJoCo Gateway G1–G7 全部验收通过

## 1. 目标

P3 将 Provider 的“执行完成”与机器人任务的“真实成功”分离。路线要求加入
`HarnessKind::Robot`、WorldModel production port、expected outcome、OutcomeVerifier、
retry/replan/recovery 与 EmbodiedEpisode（路线文档 `:725-734`）。当前 `HarnessKind` 只有 Linear
（`crates/cognit/src/harness/mod.rs:43`），WorldModel 仍是 Cognit 内部状态结构
（`crates/cognit/src/core/world_model.rs:28`），因此不得把 P3 伪装成 Linear prompt 扩展。

## 2. 固定裁决

### 2.1 ExpectedOutcome

Fabric 持有稳定的通用谓词 DTO：

```text
Predicate = Equals | NotEquals | Range | Change | All | Any
ExpectedOutcome = predicate + freshness_ms + stable_window_ms + timeout_ms
```

`path` 使用受控点分路径读取 JSON observation；禁止脚本、正则、JSONPath、自然语言判定。数值采用
有限 `f64`；NaN/Infinity、空组合、重复深度超过 8、节点超过 64 全部拒绝。

### 2.2 结果语义

- `SkillOutcome::Succeeded`：Provider 执行流程结束；
- `VerificationDecision::Matched`：新鲜世界状态满足 expected outcome；
- 最终 operation 成功必须同时满足两者；
- stale、missing、type mismatch、证据不足均 fail closed。

### 2.3 RobotHarness 状态机

```text
Observe -> Plan -> Authorize -> Execute -> Verify
                                      |       |
                                      |       +-> Matched -> Settle
                                      |       +-> RetryableMismatch -> Retry(最多1次)
                                      |       +-> ReplannableMismatch -> Replan(最多1次)
                                      |       +-> Unsafe/Unknown -> SafeStop
                                      +-> transport/provider failure -> Recovery
```

Retry 生成新的 attempt/operation，不复用 terminal operation。Replan 必须重新授权。超过边界进入
SafeStop，不继续让模型决定是否停止。

### 2.4 WorldModel

WorldModel production port 由 Fabric 定义，Executive 组合实现；它只保存每实体/设备最新的低频
规范化 observation、sequence、freshness 和 provenance。原始图像、rosbag、高频 joint stream 仅以
EvidenceRef 引用，不写入认知上下文。

### 2.5 EmbodiedEpisode

Mnemosyne 持久化不可变 Episode：goal、plan、expected outcome、attempts、前后 observation、Provider
result、verification、recovery、evidence。当前状态权威仍是 WorldModel；Episode 是历史经验，不反写
当前状态。

## 3. 依赖方向

```text
fabric: predicates, verification DTO, world-model port, episode DTO
metacog: deterministic OutcomeVerifier
cognit: RobotHarness state machine and planner-facing contracts
mnemosyne: episode repository
executive: composition, authorization, execution, settlement
hardware/corpus: unchanged P2 ports/tools
```

Metacog 不调用机器人，Cognit 不依赖 Hardware，Mnemosyne 不拥有当前世界状态。

## 4. 验收

1. Provider success + no state change => mismatch；
2. stale/missing evidence => Unknown，随后 SafeStop；
3. retry/replan 均不超过一次且产生新 attempt；
4. Robot config 只有真实 factory 完成后才可解析；
5. Linear 行为字节/生命周期兼容；
6. Episode 可重放验证裁决；
7. 使用 P2 MuJoCo 完成“有限移动—状态变化—settlement”和“假成功—无变化—恢复”两条 E2E。

## 5. 非目标

不实现视觉理解、VLA、真机、任意表达式语言、多机器人协作或学习型 verifier。
