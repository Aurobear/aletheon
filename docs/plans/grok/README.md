# Grok Build 可借鉴机制研究索引

> 文档性质：源码研究、差距分析与迁移建议，不代表已经批准的实现计划。

## 1. 研究基线

- Grok Build 本地仓库：`/home/aurobear/Bear-ws/grok-build`
- Grok Build 仓库提交：`98c3b2438aa922fbbe6178a5c0a4c48f85edc8ce`
- Grok Build 上游快照：`SOURCE_REV=124d85bc5dc6e7805560215fcc6d5413944920e1`
- Aletheon 对比提交：`bec15695860cd6b9a1b3bbf4f3c7b56ec95f8512`
- 分析日期：2026-07-17

Grok Build 是包含 TUI、headless、stdio/ACP、leader、工具、工作区、记忆和子 Agent 运行时的完整 Rust coding-agent。其仓库边界和主要 crate 职责见 `/home/aurobear/Bear-ws/grok-build/README.md:95-106`，多入口能力见 `/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-pager-bin/src/main.rs:28-47`。

## 2. 总体结论

不建议把 Grok Build 整体嵌入或替换 Aletheon。Aletheon 已经拥有更适合自身目标的 AgentControl、Kernel admission、受治理 capability、Agora、Mnemosyne 和 Conscious-core 权威边界：

```text
Grok Build                              Aletheon
-------------------------------         ----------------------------------
成熟 coding-agent 交互/runtime 机制  --> 补强宿主体验与运行时工程能力
工具流、队列、回退、ACP、folder trust   不替换领域权威、治理和认知架构
```

Aletheon 的 AgentControl 已定义 spawn/wait/send/cancel/inspect/list 权威端口（`crates/fabric/src/types/agent_control.rs:506-528`），生产服务同时持有 repository、admission、runtime registry、event spine 和 agent memory vault（`crates/executive/src/service/agent_control/mod.rs:100-114`）。因此应采用“移植机制、保持权威”的策略。

## 3. 建议优先级

| 优先级 | 借鉴项 | Aletheon 收益 | 建议 |
|---|---|---|---|
| P0 | Folder Trust / repo-local 配置门控 | 允许任意 cwd，同时隔离不可信仓库的 hooks/MCP/plugins/LSP | 优先做 |
| P0 | 流式工具执行事件 | 长命令进度、取消、TUI 反馈、远程运行可观测性 | 优先做 |
| P1 | Prompt Queue + mid-turn interjection | 多客户端、多用户输入排序和安全插话 | 紧随 P0 |
| P1 | Turn workspace checkpoint/rewind | 失败恢复、用户撤销、Agent 试错安全 | 分域渐进启用 |
| P1 | Typed lifecycle contributors | 降低 Executive/Cognit/Corpus hook 耦合 | 保持 host 拥有循环 |
| P2 | 子 Agent 后台资源结算与 reparent | 避免子 Agent 退出后任务丢失或泄漏 | 接入 AgentControl |
| P2 | ACP adapter | IDE/编辑器生态和标准客户端接入 | 作为边缘适配器 |
| P2 | OS 级沙箱执行 | Landlock/Seatbelt/bwrap 内核强制沙箱，deny glob，子进程网络隔离 | 与 Folder Trust 组合，不替换 WorkspacePolicy |
| P3 | 混合记忆检索细节 | FTS + vector 降级、凭证端点约束 | 只吸收策略，不替换 Mnemosyne |
| P3 | 上下文压缩引擎 | 三种压缩策略（全量替换/尾部保留/分块），trait seam 解耦 | 对接 Conscious Context Slot，先做全量替换 |

## 4. 文档导航

1. [总体架构与边界](01-architecture-and-boundaries.md)
2. [任意工作目录、Folder Trust 与多用户隔离](02-folder-trust-and-multi-user-workspaces.md)
3. [流式工具运行时](03-streaming-tool-runtime.md)
4. [Prompt Queue 与中途插话](04-prompt-queue-and-interjection.md)
5. [Typed lifecycle extensions](05-typed-lifecycle-extensions.md)
6. [Workspace checkpoint 与 rewind](06-workspace-checkpoint-and-rewind.md)
7. [子 Agent 资源继承、结算与恢复](07-subagent-resource-settlement.md)
8. [ACP 与多入口适配](08-acp-and-runtime-adapters.md)
9. [记忆检索与凭证安全](09-memory-search-and-credential-safety.md)
10. [采用路线图与决策清单](10-adoption-roadmap.md)
11. [OS 级沙箱执行](11-sandbox-enforcement.md)
12. [上下文压缩引擎](12-compaction-engine.md)

## 5. 使用规则

- 这些文档中的“建议类型/接口”均为候选设计，不是当前代码事实。
- 任何实施前都应重新读取对应 Grok 源码与 Aletheon 当前分支，因为两边代码都可能变化。
- 不复制 Grok 的整体运行时、AgentControl 替代物或 memory authority。
- 若直接复制或改写 Apache-2.0 源码，必须完成许可证、NOTICE 和修改声明审查。Grok 第一方代码许可证证据见 `/home/aurobear/Bear-ws/grok-build/README.md:127-139`；Aletheon 当前为 MIT（`Cargo.toml:19-23`）。

