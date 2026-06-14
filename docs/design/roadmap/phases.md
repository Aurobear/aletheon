# 实现路线图 (Implementation Roadmap)

> 统一 6 Phase 定义，每个阶段独立交付价值。
>
> **Phase 1-4 已全部实现，Phase 5 已实质完成（eBPF mock、向量记忆、FUSE real mount、Split Sandbox、Container Sandbox、Integrity Monitor），Phase 6 部分实现（自动化系统已实现，多设备/内核 IPC 延后）。B1-B5 实现批次 + 设计改进 PR 均已合并。** 下方为各 Phase 的设计规格，实现代码在 `crates/aletheon-*/src/`。

### Phase 1: 最小可用 Agent (2-3 周)

> **建议拆分**: Phase 1 可进一步拆为 1a（内存中的 ReAct 循环 + 基本工具 + CLI，无持久化）
> 和 1b（加入 session 持久化与崩溃恢复）。1a 可在 1 周内验证核心循环假设，
> 1b 再补齐状态管理。

```
目标: 能在 Arch Linux 上跑起来的 agentd

├── 项目骨架搭建
│   ├── Cargo workspace (aletheon-abi, aletheon-comm, aletheon-memory, aletheon-body, aletheon-self, aletheon-brain, aletheon-runtime, aletheon-meta, aletheond, aletheon-cli)
│   ├── 目录结构
│   └── CI/CD 基础
│
├── 核心引擎
│   ├── ReAct 推理循环 (Anthropic SDK 模式)
│   ├── Content-block 消息协议
│   ├── 本地 llama.cpp 集成
│   ├── 上下文压缩 (compaction)
│   └── 基础工具注册
│
├── 工具系统
│   ├── Tool trait + 注册表
│   ├── bash_exec, file_read, file_write
│   ├── system_status, process_list
│   └── 权限分级 (L0-L3)
│
├── 系统服务
│   ├── systemd service 定义
│   ├── 配置文件 (分层配置栈)
│   ├── 日志系统 (tracing)
│   └── CLI 客户端 (agent-cli)
│
├── 会话持久化 (P0)
│   ├── SQLite 会话存储
│   ├── Checkpoint 机制
│   └── 崩溃恢复
│
└── 验收: agent-cli 能对话，能执行 bash 命令，重启后恢复会话
```

### Phase 2: 感知 + 记忆 (3-4 周)

```
目标: Agent 能"看见"系统并记住事情

├── 感知引擎
│   ├── eBPF 感知模块 (文件/进程/网络)
│   ├── /proc /sys 轮询
│   ├── journald 日志流
│   ├── inotify 文件监控
│   ├── 事件聚合与去重
│   └── 感知事件 → Core Memory 自动注入
│
├── 记忆系统 (Letta 模式)
│   ├── Core Memory (Block 结构)
│   ├── Recall Memory (SQLite)
│   ├── Archival Memory (LanceDB)
│   ├── 上下文预算追踪
│   └── 记忆工具 (append/replace/search)
│
├── 工具输出管理 (P0)
│   ├── 输出大小限制
│   ├── 溢出到文件
│   └── 截断预览
│
└── 验收: Agent 能感知系统变化并记住用户偏好
```

### Phase 3: 沙箱 + 安全 + FUSE (2-3 周)

```
目标: 安全地执行命令，提供文件系统接口

├── 沙箱执行
│   ├── bubblewrap 沙箱
│   ├── seccomp 过滤
│   ├── landlock 策略
│   ├── cgroups 资源限制
│   ├── WritableRoot 只读子路径保护
│   └── 审计日志 (audit.jsonl)
│
├── 安全引擎
│   ├── 策略引擎 (分层配置)
│   ├── 回滚引擎 (btrfs snapshot + 文件备份)
│   ├── 权限升级确认流程
│   └── 循环检测 Guardrail (P0)
│
├── FUSE 接口
│   ├── /mnt/agent/context/
│   ├── /mnt/agent/controls/
│   ├── /mnt/agent/sensors/
│   └── /mnt/agent/logs/
│
└── 验收: 命令在沙箱中执行，FUSE 可访问，循环自动阻断
```

### Phase 4: 多 Agent 编排 (3-4 周)

```
目标: 多个专业 Agent 协作

├── 编排引擎 (AutoGen + CrewAI 模式)
│   ├── Agent 注册表 + 能力声明
│   ├── Selector 编排 (LLM 选择 Agent)
│   ├── DelegateTool (委托即工具)
│   ├── 可组合终止条件
│   ├── 子 Agent 独立预算
│   └── 安全护栏 (Guardrail)
│
├── 专业 Agent
│   ├── fs_agent, net_agent, proc_agent
│   ├── code_agent, coordinator
│   └── Hook 系统 (P1)
│
├── 工具增强 (P1)
│   ├── 工具分层暴露 (Direct/Deferred/Hidden)
│   ├── 工具并行执行
│   └── MCP 集成
│
├── IPC: Unix socket
│
└── 验收: 复杂任务自动分解到多个 Agent 协作完成
```

### Phase 5: 内核 IPC 加速 (2-3 周)

> **状态: 🟢 实质完成** — 内核模块未实现（用户态 IPC 已满足需求），其余核心能力均已落地。
> 用户态 IPC（Unix socket）已满足需求，内核模块为可选加速。

```
目标: Agent 间零拷贝通信 (可选加速)

├── agent_ipc.ko                               ← ❌ 未实现（用户态 IPC 已满足需求）
│   ├── Agent Ring (类似 io_uring)
│   ├── 共享内存 ring buffer
│   ├── 优先级消息队列
│   └── 用户态 API 封装
│
├── 系统调用                                    ← ❌ 未实现
│   ├── sys_agent_register
│   ├── sys_agent_send / recv
│   └── sys_agent_share_mem
│
├── DKMS 打包                                   ← 设计完成，未实现
│
├── 自动降级: 有内核模块用内核，没有用 Unix socket  ← ✅ IpcManager 已实现自动探测
│
└── 验收: Agent 间通信延迟 <10μs               ← 当前 Unix socket 延迟 ~100μs
```

**Phase 5 已完成项:**
- eBPF 感知: mock /proc 回退可用（sched/net/block），真实 eBPF ring buffer 未实现
- 向量记忆: Qdrant 后端可用（需 feature flag），BM25 + TF-IDF 工具搜索已实现
- FUSE: fuse3 真实挂载已实现（FUSE AgentFs real mount）
- Split Sandbox: SplitSandbox（bwrap + fallback chain）已实现
- Container Sandbox: ContainerSandbox 已实现（B5）
- Integrity Monitor: IntegrityMonitor 已实现（B5）
- io_uring IPC: 代码存在（feature gate），默认未启用

### Phase 6: 高级功能 (持续)

```
├── DiGraph 编排 (DAG 工作流)              ← 已实现 (orchestration/digraph/)
├── 云端推理 fallback                       ← 代码已实现 (inference/router.rs)，但未接入 Engine
├── Android 平台适配                        ← 已实现 (platform/android.rs)
├── 自动化系统                              ← 已实现 (automation/，B4 + design improvements)
├── MCP OAuth 2.0                          ← 已实现 (mcp/auth.rs，B5)
├── MCP Transports (StreamableHTTP + SSE)  ← 已实现 (B4)
├── 嵌入式 SDK                              ← 未实现
├── 多设备记忆同步                           ← 延后
├── 内核 IPC (agent_ipc.ko)               ← 延后（用户态 IPC 已满足需求）
├── 统一内存优化 (NVIDIA)                    ← 未实现
├── 可观测性 (Prometheus metrics, debug CLI) ← 部分实现 (observability/ 模块)
└── 生态建设 (插件系统、技能市场)              ← 部分实现 (plugin/ 系统)
```

**Phase 6 剩余未实现项:** 嵌入式 SDK、多设备记忆同步（延后）、内核 IPC（延后）、NVIDIA 统一内存、技能市场。

### 实现批次 (B1-B5) + 设计改进

所有实现批次和设计改进 PR 均已合并到 dev 分支:

| 批次 | PR | 内容 |
|------|-----|------|
| B1 | #99 | Security foundation — ErrorHandling, WritableRoot, SelfProtection |
| B2 | #100 | Tool enhancement + observability — 6 modules, 98 tests |
| B3 | #102 | Inference + memory — 4 modules, 43 tests |
| B4 | #103 | Platform integration — Panic Recovery, Boot, Awareness, FUSE, MCP Transports |
| B5 | #104 | Advanced features — IntegrityMonitor, MCP OAuth 2.0, Container Sandbox |
| Design | #105 | Design improvements — Memory Pipeline, Split Sandbox, Automation |
