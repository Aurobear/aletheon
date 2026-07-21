# Aletheon Production Readiness Hardening Plan

> **Date:** 2026-07-21（2026-07-22 校正）
> **Status:** Active；H0–H1 已完成，H2 进行中
> **Role:** 唯一可执行 hardening 队列
> **Companion evidence:** `docs/plans/2026-07-21-aletheon-coupling-and-external-interface-audit.md`
> **Accepted baseline:** `docs/deployment/ser8-acceptance-2026-07-21.md`

## 1. 目标与范围

SER8 单机验收已经证明真实 inference、Pi、coding diff、durable memory 与 systemd
定时闭环可运行。本计划不重复该验收，而是把“单台审计主机可用”推进到“可以扩大部署”的
剩余风险整理成一条有依赖、有停止条件、可逐批提交的工作队列。

本计划遵循两个范围约束：

1. 优先使用现有 crate 边界；允许在现有 crate 内新增明确的 port/type/composition unit；
2. 不默认新增 crate，也不承诺“零架构边界变化”。若某批需要新 crate 或跨领域公共抽象，
   必须先单独形成架构决策，不能在 hardening 提交中顺带引入。

## 2. 事实基线与已校正结论

本节来自当前工作树的静态代码复核。计数只用于定位，不能替代逐调用链验证。

| 主题 | 当前代码事实 | 本计划采用的结论 |
|---|---|---|
| 配置优先级 | system → user → project → `ALETHEON__` environment → CLI（`crates/executive/src/core/config/mod.rs:259-301`） | 优先级已定义；问题是业务模块存在绕过 typed config 的直接 env 入口 |
| Provider | 存在两个 `ProviderConfig` 与两条 factory/registry 路径（`crates/cognit/src/config/mod.rs:683`、`crates/cognit/src/impl/inference/provider_config.rs:12`、`crates/cognit/src/impl/provider_registry.rs:149-188`、`crates/cognit/src/impl/llm/provider_factory.rs:28-106`） | P0 行为分叉风险，先收敛单一真源 |
| GBrain migration | 每次 open 都执行幂等建表/补列/回填，最后写 `user_version`；无显式总事务（`crates/mnemosyne/src/backends/gbrain/migrations.rs:7-145`） | 不存在“重开因版本跳过迁移”；事务化是 P1 韧性增强 |
| Session schema | DB 建表无 `user_version`，但 session/item 记录校验 `SESSION_SCHEMA_VERSION`（`crates/executive/src/impl/session/canonical_store.rs:29-74`） | 缺少数据库结构迁移版本，不是完全没有 schema version |
| 网络策略 | 默认拒绝及 host/protocol/port/DNS 检查已存在（`crates/fabric/src/types/network_policy.rs:53-104`） | 私网/metadata 拒绝是策略开放后的条件性 P1，且需覆盖 DNS 解析与重定向 |
| LLM 瞬态错误 | scheduler 对 429、5xx、network、timeout、`eof` 做有界重试（`crates/cognit/src/impl/llm/scheduler.rs:31-74`） | 网络 EOF 已有恢复；Display 字符串分类是 P2 脆弱点 |
| Health | daemon 已有 health RPC，SER8 运维验收已调用 | 不新增重复 `/health`；补外部依赖 degraded/readiness 聚合 |

### 2.1 不得再作为实施依据的旧说法

- “配置 precedence 未定义”；
- “GBrain 重开会因 `user_version` 跳过未完成迁移”；
- “SessionStore 完全没有 schema versioning”；
- “没有每次 open 的 `integrity_check` 就等于断电必然损坏”；
- “approval oneshot send 失败必然导致 turn 挂起”；
- “所有 `/tmp` 字面量都必须迁移”；
- “持久化是当前最高 P0”，除非先获得可复现损坏证据。

## 3. 风险等级与批次规则

| 等级 | 定义 |
|---|---|
| P0 | 已验证或可由不可信输入触发，可能导致 daemon 崩溃、权限绕过、行为分叉或不可接受的核心闭环失败 |
| P1 | 生产韧性、可观测性或开放策略下的安全风险；需要排期，但没有证据支持阻断所有部署 |
| P2 | 可维护性、长期容量、诊断或类型质量改进 |

每一批必须满足：

1. **先复现后修改**：风险描述能落到具体符号、输入与失败行为；
2. **边界明确**：列出涉及文件、外部行为变化和明确非目标；
3. **独立提交**：不混入后续架构拆分；
4. **确定性验证**：使用 `bash scripts/cargo-agent.sh ...`，禁止直接运行 `cargo`；
5. **停止条件**：若前置事实不成立，降级或取消批次，而不是为了完成计划制造改动。

## 4. 唯一实施队列

当前执行记录：

| 批次 | 状态 | 证据 |
|---|---|---|
| H0 | **PASS** | `docs/deployment/hardening-baseline-2026-07-22.md` |
| H1 | **PASS** | `docs/deployment/hardening-h1-external-input-2026-07-22.md` |
| H2–H11 | Pending | 按下列依赖顺序推进 |

```text
H0 事实基线
  -> H1 外部输入 panic 复现与修复
  -> H2 Provider 单一真源
  -> H3 typed config / secret preflight
  -> H4 后台任务监督（MCP first）
  -> H5 SQLite migration resilience
  -> H6 coding apply / settle 错误契约
  -> H7 OutboundTransport / SSRF
  -> H8 路径治理
  -> H9 Pi 治理证据
  -> H10 real coding e2e
  -> H11 Executive composition
```

H0–H3 是扩大部署前的主线；H4–H10 可按依赖逐批推进；H11 是后续架构收敛，不能阻塞
已经通过验收的单机闭环，也不能与功能 hardening 混成一个大提交。

## 5. 批次定义与验收

### H0：刷新事实与架构基线

**目标**

- 更新旧依赖图中 Corpus、Execd、MCP ownership 的过时结论；
- 为两份本文档中的代码锚点做可复现 grep/read 清单；
- 确认当前 deployment acceptance、脚本和 health RPC 的实际入口。

**验收**

- 旧文档与当前 Cargo/code 不再对“现状”给出冲突答案；
- 文档引用不指向已删除计划；
- 所有后续批次均能指向当前存在的符号和测试目标。

**停止条件**：若 acceptance 记录不能由当前脚本复现，先修复运维基线，不进入架构改造。

### H1：验证并修复外部输入 panic（候选 P0）

候选范围：

- `crates/executive/src/impl/channel/daemon_adapter.rs`；
- `crates/executive/src/impl/channel/gmail/goal_draft.rs`；
- `crates/executive/src/impl/gbrain/mcp_adapter.rs`；
- 相邻 ingest/parser 边界。

**步骤**

1. 逐个定位“值来自远端/持久化数据”的 `unwrap`/`expect`，排除锁中毒与已验证不变量；
2. 为每个保留候选记录具体 `path:line`、输入形状和预期 reject 行为；
3. 先写 failing regression，证明 malformed input 可穿透到 panic；
4. 使用现有 error/reject 类型修复，daemon 保持存活并输出不含敏感数据的诊断。

**验收**

- 至少一个已复现的远程/持久化输入 panic 被回归测试覆盖并消除；
- malformed input 产生结构化 reject，不创建半完成 goal/session；
- 若没有候选可复现，本批降为“审计完成、无 P0 代码变更”，不得按 grep 数量批量改写。

### H2：Provider 配置与创建单一真源（P0）

**现状证据**

- 应用 schema：`crates/cognit/src/config/mod.rs:683`；
- inference schema：`crates/cognit/src/impl/inference/provider_config.rs:12`；
- Registry 路径：`crates/cognit/src/impl/provider_registry.rs:149-188`；
- Factory 路径：`crates/cognit/src/impl/llm/provider_factory.rs:28-106`。

**目标行为**

- 一个 canonical provider definition；
- 一个生产创建入口；
- transport 显式声明为权威，`Auto` 仅作为受测兼容模式；
- timeout、token/context、pricing 与 credential identity 由同一路径解释；
- OpenAI、Anthropic、Ollama 不再因调用入口不同而得到不同参数。

**验收**

- 同一配置从所有公开入口得到相同 provider kind 与运行参数；
- URL heuristic 有明确兼容测试，或从生产决策路径删除；
- 重复 `ProviderConfig`/secret-name 拼接不再存在生产双实现。

### H3：typed config、凭据引用与启动预检（P0/P1）

**现状证据**

- layered config 已有明确顺序（`crates/executive/src/core/config/mod.rs:259-301`）；
- runtime legacy env 仍直接覆盖（`crates/executive/src/core/runtime_core.rs:69-84`）；
- Google bootstrap 直接解析部署变量（`crates/executive/src/impl/daemon/bootstrap/google.rs:37-124`）。

**范围**

- 只迁移业务配置和 secret 引用；保留 systemd、XDG、display、credential directory 等 host
  protocol env；
- optional integration 只有在“已启用”时才要求其必需字段；Google `client_secret` 是否必需
  由实际 OAuth client type 决定，不能一概强制；
- 启动诊断只显示配置来源与缺失引用，不打印 secret value。

**验收**

- enabled integration 缺少必需配置时，在启动核心工作前返回明确 typed diagnostic；
- disabled/不需要 secret 的模式不被误拒绝；
- domain service 不新增直接业务 `std::env` 读取；
- 旧变量若保留，具有明确 precedence、deprecation 和测试。

### H4：后台任务监督，MCP first（P1）

**现状证据**：`crates/corpus/src/tools/mcp/client.rs:847,856,933,1098,1121` 等生产
`tokio::spawn` 缺少共同监督契约。

**范围**

- 先覆盖 MCP health/reconnect；登记任务名、终止原因、取消信号和有界 shutdown；
- panic/异常退出投影为 degraded health，并决定是否重启；
- 不一次性包装全仓库所有 spawn；reasoning logger、perception 根据同一模式后续迁移；
- `crates/mnemosyne/src/service.rs:791` 的同步 DB 工作先压测，再决定 `spawn_blocking` 或 DB worker。

**验收**

- 故障注入能观察 MCP supervisor 异常、健康降级与恢复/停止决策；
- shutdown 不遗留新任务，且等待有超时；
- 正常 reconnect 行为无回归。

### H5：SQLite migration resilience（P1）

**范围**

- GBrain migration：验证 rusqlite/SQLite 对当前 DDL 的事务行为，添加中断/重入测试，再决定
  显式事务边界；
- SessionStore：为数据库结构引入可演进版本策略，同时保留已有 record schema 校验；
- 完整性诊断：单独评估离线 `quick_check` 或受控启动检查，不默认每次 open 跑全量
  `integrity_check`。

**验收**

- 在每个迁移步骤模拟失败后，重新打开能继续完成或明确 fail closed；
- 版本只在对应 schema/data 变更成功后推进；
- 旧 session DB 与 record schema 行为均有 fixture 测试；
- 没有性能证据时，不把完整性扫描放到高频 open 路径。

### H6：coding apply / settle 错误契约（P1）

候选范围：

- `crates/executive/src/impl/approval/apply_coordinator.rs`；
- `crates/executive/src/impl/goal/attempt_coordinator.rs`；
- `crates/executive/src/impl/goal/coordinator.rs`；
- `crates/executive/src/service/turn_pipeline.rs:135,148`；
- `crates/executive/src/service/admin_service.rs:156,206`。

**验收**

- 对每个被丢弃的结果明确 best-effort、warn-and-continue 或 propagate；
- apply/settle 失败保留 attempt history、diff hash 与可重试状态；
- oneshot receiver 已关闭只记录正确终态，不虚构“必然挂起”；
- 回归测试覆盖重复 apply、consumer 消失和事件发布失败。

### H7：出站治理与条件性 SSRF 防护（P1）

**范围**

- 在现有 crate 内定义最小 endpoint/policy/transport port；不创建“万能 ExternalService”；
- 先迁移 MCP/Google，再依据收益评估 Telegram、automation、embedding、LLM；
- 保留 adapter 业务协议与 retry 语义，统一 host authority、timeout 上限、TLS/redirect、错误分类和健康；
- 在 DNS 解析后及每次 redirect 后校验最终 IP，覆盖 loopback、link-local/metadata、RFC1918、
  IPv6 ULA/loopback 与 DNS rebinding。

**验收**

- 默认 deny 不回归；显式 allow-host 仍不能通过重定向或重绑定访问禁止地址；
- 本地开发所需 loopback 必须通过显式 trust class/authority 放行，而非全局例外；
- credential 只在 endpoint identity 获批后解析，日志不暴露 secret。

### H8：运行路径分类与 XDG 收敛（P1/P2）

先对 `/tmp`、socket、cache、worktree、测试 tempdir 做逐点分类；测试 fixture、受控临时文件与
协议默认不能仅凭字符串计数迁移。生产持久数据进入 XDG data，短生命周期 socket/lock 进入
XDG runtime，cache 可清理，secret 不落普通临时文件。

**验收**

- 每个生产路径有 owner、生命周期、权限和 cleanup 契约；
- 多实例不会因共享固定路径碰撞；
- 迁移有兼容/清理说明，测试 tempdir 不被误改。

### H9：Pi 治理证据（P1）

当前 Pi 已有真实 exit/elapsed/token/cost/diff hash；剩余工作只针对可验证缺口：
capability audit 不能用空向量占位，`diff_artifact` 应与已有 diff evidence 建立稳定引用。

**验收**

- capability evidence 来自实际可观测信号；做不到时明确标记 unavailable，不能伪装 present；
- diff artifact 与 `diff_sha256` 一致，受大小与敏感信息策略约束；
- env-gated real Pi contract 与 fixture contract 均运行并能发现协议漂移。

### H10：真实 coding e2e（P1）

在 disposable repository 中验证：goal → Pi/fixed executor contract → real verifier → approval →
hash-bound apply → settled evidence。为了确定性，常规测试可以使用固定 executor，但 verifier 与
`git apply` 必须走生产实现；真实 Pi 另作为 env-gated/定时 gate。

**验收**

- 成功路径生成并应用预期 diff；
- hash 不一致、verify 失败、重复 consume 都 fail closed；
- 测试使用 `bash scripts/cargo-agent.sh` 约束构建资源；
- CI/定时运行策略在当前 workflow 中有真实配置，不引用已删除设计文档。

### H11：Executive composition 收缩（P2，独立架构阶段）

以 `crates/executive/src/impl/daemon/bootstrap/request.rs:66` 为入口，将 inference、memory、
integrations、agents、tools、sessions 逐个提取为有 typed 输入/输出的 composition unit。
该阶段不得重新读取全局 env，不以“减少行数”为验收，也不顺带拆所有大文件。

**验收**

- 顶层 bootstrap 只表达构造顺序和资源传递；
- optional integration 的失败/降级契约可独立测试；
- 每次提取保持外部行为不变并单独提交；
- 是否拆 MCP/security/settlement 大模块由后续状态机证据决定。

## 6. 验证矩阵

所有 Rust 命令必须经过仓库 wrapper：

```bash
bash scripts/cargo-agent.sh test -p PACKAGE NARROW_TARGET
bash scripts/cargo-agent.sh clippy -p PACKAGE --all-targets -- -D warnings
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/architecture-check.sh
git diff --check
```

| 批次 | 最小确定性验证 | 扩大部署前证据 |
|---|---|---|
| H1 | malformed input regression + daemon-survives assertion | 不可信输入不能触发复现 panic |
| H2 | provider definition/factory table tests | 真实 OpenAI/Anthropic/Ollama smoke（按已配置能力） |
| H3 | config precedence + enabled/disabled integration table tests | dry-run/preflight 不泄露 secret |
| H4 | supervisor failure injection + shutdown timeout test | MCP 断线、恢复、健康降级演练 |
| H5 | migration interruption/reopen fixtures | 旧数据库副本升级演练 |
| H6 | apply/settle failure matrix | 重启后 attempt/evidence 一致 |
| H7 | reserved IP、redirect、DNS-rebind deterministic tests | 受控外部 endpoint smoke |
| H8 | multi-instance path tests | systemd 用户服务重启与清理 |
| H9 | Pi receipt contract tests | real Pi env-gated gate |
| H10 | disposable repo e2e | 定时 real Pi coding e2e |
| H11 | per-composition construction/failure tests | 架构检查与现有 acceptance 回归 |

workspace-wide check 只能由集成/验证 owner 串行运行；不得并发执行 `executive` 或 workspace build。

## 7. 提交、回滚与推进规则

每批应独立提交，并包含问题/方案/验证上下文。推进下一批前：

1. 检查 staged diff，确认没有混入用户删除的旧计划或其他工作树修改；
2. 运行该批最小测试及 `git diff --check`；
3. 记录外部行为变化与回滚方法；
4. 若测试发现前提错误，先校正文档和优先级，再决定是否继续；
5. 在全量验证通过前，不把“代码已合并”写成“生产闭环已验证”。

回滚优先按独立提交反向恢复；涉及 DB migration 的批次必须先证明旧二进制兼容或明确不可降级，
不能把数据库文件删除作为默认回滚方案。

## 8. 非目标与后续项

- IntentClassifier 的 ML/数据驱动替换不属于本轮 hardening；
- hardware production wiring 继续保持 experimental，除非另有明确需求；
- unsafe 清账、占位 feature、事件表 TTL 需要独立证据，不因静态计数自动升级；
- 不为了统一而创建无 owner 的 `common`/`external-types` crate；
- 不在本文维护第二套架构现状，事实证据统一回写 companion audit。

## 9. 完成定义

本计划完成不等于所有 P2 重构完成。扩大部署所需的最小完成条件是：

- H0 基线可复现；
- H1 的 P0 候选已经复现并修复，或经测试证伪并降级；
- H2 消除 Provider 行为分叉；
- H3 对启用集成实施 typed preflight 且不泄密；
- H4–H10 中与目标部署实际启用能力相关的批次通过对应验证矩阵；
- SER8 acceptance 在变更后重新通过，健康状态能区分 core ready 与外部依赖 degraded；
- 没有未解释的数据库不可逆变化，也没有依赖已删除文档的验收项。
