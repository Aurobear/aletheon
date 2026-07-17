# Grok-inspired Agent Runtime Hardening：采用路线图

## 1. 推荐组合

建议把可借鉴项组织成一个 umbrella program，但拆成相互独立的设计与实施计划：

```text
G1 Folder Trust / multi-user workspace
        |
        +--> G3 Prompt Queue / Interjection
        |
G2 Streaming Tool Runtime ------> G8 ACP Adapter
        |
        +--> G5 Lifecycle Contributors
        |
G4 Workspace Checkpoint/Rewind --> G6 Subagent Settlement

G7 Memory Search Hardening（独立，仅借鉴策略）
```

## 2. 依赖与优先级

| ID | 项目 | 依赖 | 风险 | 优先级 |
|---|---|---|---|---|
| G1 | Folder Trust 与多用户 trust receipt | WorkspacePolicy/principal | 中 | P0 |
| G2 | Streaming Tool Runtime | governed capability/turn event | 中高 | P0 |
| G3 | Prompt Queue/Interjection | session authority、持久化 | 中高 | P1 |
| G4 | Workspace Checkpoint/Rewind | workspace identity、leases | 高 | P1 |
| G5 | Typed Lifecycle Contributors | stable turn/session hook points | 中 | P1 |
| G6 | Subagent Resource Settlement | AgentControl、G2、G4 部分能力 | 高 | P2 |
| G7 | Memory Search/Credential Hardening | Mnemosyne current plans | 中 | P3 |
| G8 | ACP Adapter | G2、G3、approval mapping | 中高 | P2 |

## 3. 每项的最小可交付切片

### G1

- 只做 repo-local executable config discovery。
- 任意 cwd 保持可启动。
- interactive prompt + headless restricted。
- trust receipt 绑定 principal + workspace + digest。

### G2

- Fabric 定义 progress/terminal invariant。
- 旧工具自动 terminal-only。
- 首个 streaming tool 选择 terminal/bash。
- bridge 到现有 turn event stream。

### G3

- 先做持久 FIFO queue、stable id、version conflict。
- 再做 safe-point interjection。
- 不在第一版做任意 reorder 或跨用户协同编辑。

### G4

- 第一版仅 FS、单 Agent、用户显式触发。
- durable/git/multi-agent 分开后续计划。

### G5

- 第一版只迁移 session start/end、turn start/end、tool terminal。
- contributor 只产生 bounded declarative effects。

### G6

- 先建立 resource inventory + idempotent settlement receipt。
- 再做 background task reparent。

### G7

- 先做 endpoint-scoped embedding credential 和 FTS fallback 测试。
- vector/MMR 优化必须服从 Mnemosyne authority/scope。

### G8

- 第一版只做 ACP initialize/new/prompt/cancel/notification。
- permission、FS、terminal 分阶段接入。

## 4. 明确不做

- 不 vendoring 整个 Grok Build workspace。
- 不用 Grok session/subagent coordinator 替换 AgentControl。
- 不用 Grok permission mode 替换 Executive authority。
- 不用 Grok memory store 替换 Mnemosyne。
- 不把 Grok leader/update 机制移植到 Aletheon daemon。
- 不因 ACP/TUI 需要把客户端协议类型渗透进 Cognit、Dasein、Agora。

## 5. 共同工程约束

### 5.1 多用户

所有 queue、trust、approval、checkpoint、notification、memory query 都必须绑定 principal/session/thread，而不是使用进程全局状态。Aletheon 已有 principal/thread/turn/workspace trusted context（`crates/executive/src/service/governed_capability.rs:20-34`），新功能应复用而非另建字符串身份。

### 5.2 权威终态

Tool call、turn、Agent attempt 各自只能有一个 terminal truth。progress、notification、client disconnect 都不能暗自改变成功/失败。

### 5.3 恢复

所有持久状态更新必须可幂等重放；queue consume、checkpoint finalize、resource settlement、memory promotion 都要有 idempotency key/receipt。

### 5.4 有界性

队列长度、interjection bytes、tool progress buffer、checkpoint disk、background resources、memory candidates 全部必须有硬上限和清理策略。

### 5.5 许可证

优先重新实现接口和语义。若复制/改写 Grok Apache-2.0 代码，需逐文件记录来源、变更，并更新第三方 notices；许可证事实见 `/home/aurobear/Bear-ws/grok-build/README.md:127-139`，Aletheon 为 MIT（`Cargo.toml:19-23`）。

## 6. 立项前决策清单

每个子项目进入 brainstorming/spec 前，应先回答：

1. 权威 owner 是 Fabric、Executive、Kernel、Interact 还是 Mnemosyne？
2. 状态按 principal/session/thread/agent/task 哪个 scope 持久化？
3. crash 后的 authoritative recovery decision 是什么？
4. 哪些错误 fail closed，哪些允许 degraded fallback？
5. 上限、超限行为、清理策略是什么？
6. legacy runtime/tool/client 如何兼容？
7. 哪些事件需要进入 canonical event spine？
8. 是否直接复制 Apache-2.0 代码？如果是，NOTICE 怎么处理？

## 7. 推荐下一步

先分别为 G1 和 G2 创建正式设计：

- **G1** 能解决“任何目录启动”与安全加载 repo-local 配置之间的矛盾。
- **G2** 为长工具、TUI、ACP、后台资源和取消提供共同事件基础。

两者完成后再启动 G3；G4/G6 风险较高，应在现有 Conscious-core 与 AgentControl 变更稳定、全量 CI 通过后实施。

