# Aletheon 统一执行计划（2026-08-04）

> 本文件是 **架构 / UX / robot 三份计划的唯一执行权威**，也是**计划基线 SHA 的唯一声明处**。
> 自动执行按 §3 的依赖表选择节点；一个 Goal 只处理一个节点，依赖判定由 §8 的外层监督器完成，不能依赖 Goal runtime 自行推断。
> 设计契约仍以三份源计划为准；三份源计划自带的 PR 表**一律视为历史，禁止调度**，冲突以本文件为准。
> 文档状态：`approved-executing`；B0 已由 PR #162 合入，B1 已记录 owner approval 与 `external_supervisor` 控制面。
>
> 源计划（只提供设计契约，不提供执行顺序）：
> - `Aletheon_Architecture_Stabilization_and_Convergence_Plan_2026-08-04.md`（架构契约）
> - `Aletheon_User_Experience_and_Engineering_Workflow_Plan_2026-08-04.md`（产品 UX）
> - `robot-vla-production-closure-plan.md`（robot 域契约与域内关闭判定）
> - `deepseek-cache-and-message-optimization-plan.md`（provider/cache 事实 owner，见 §1）

## 0. 基线（唯一声明处）

```text
PLAN_BASELINE_REF   = origin/dev
PLAN_BASELINE_SHA   = c080b08bf3170dd8a09acdb738a255133abbf11b
PLAN_BASELINE_DATE  = 2026-08-05
LOCAL_DEV_AT_REVIEW = 278f357638bf575cda34008a176a71c036326929  # stale，不得作为开工基线
```

- 三份源计划的 baseline 行只允许写“见统一执行计划 §0”，不得各自声明 SHA。
- `PLAN_BASELINE_SHA` 是本轮设计事实的冻结点，不因本计划自己的节点逐个合入而反复改写。
- 每个节点开工前执行 `git fetch origin dev` 与 `git rev-parse origin/dev`，把结果写入该节点 STATUS 块的 `BASE`。
- 若 `origin/dev` 的新增提交全部来自本计划已经 `accepted` 的节点，直接以最新 SHA 开新分支；若包含外部或未登记提交，停止该节点，重新 grep 受影响的 `path:line` 事实并提交 doc-only reconciliation。
- 新分支必须从 `origin/dev` 创建；本地 `dev` 未 fast-forward 前禁止作为 base。
- 禁止用“比 main 超前 N 个提交”描述任何现状。

### 0.1 B0/B1：无人值守执行前置（不属于自动 Goal DAG）

B0 已将以下 **6 份文档**纳入版本控制并通过 PR #162 合入 `dev`：

```text
docs/plans/Aletheon_Unified_Execution_Plan_2026-08-04.md
docs/plans/Aletheon_Architecture_Stabilization_and_Convergence_Plan_2026-08-04.md
docs/plans/Aletheon_User_Experience_and_Engineering_Workflow_Plan_2026-08-04.md
docs/plans/robot-vla-production-closure-plan.md
docs/testing/robot-runtime.md
docs/plans/execution-status.md
```

**B0（文档 bootstrap，人工或已授权的普通开发流程执行）**：

1. 从 `origin/dev@c080b08b...` 创建独立文档分支；不得从 stale local `dev` 开工。
2. 检查并移除 `docs/testing/robot-runtime.md` 中的内部用户名、IP、绝对路径和可直接复制的隧道命令。
3. 提交上述 6 份文档，创建 PR 到 `dev`，等待 CI 通过并合并。
4. 验证合并后的 `origin/dev` 能读取本文件与 `execution-status.md`；在后者记录 B0 merge SHA。
5. 在 `execution-status.md` 写入 `APPROVED_BY`、`APPROVED_AT`、`EXECUTION_CONTROL_PLANE`；未填写不得启动循环。

**B1（控制面 preflight）**：必须明确今晚使用哪一个控制面：

- `external_supervisor`：受信任的外层监督器通过 official user socket/已审核接口一次只创建一个 Goal；这是调度手段，不是 installed product acceptance；或
- `installed_aletheon_goal`：只有生产 CLI/typed client 已提供 canonical `goal create/list/run/pause/cancel` 且安装态验收通过后才可选。

当前代码基线只有 daemon 侧 `goal.create/list/run/pause/cancel` handler；生产 `aletheon` parser 尚无 `goal` 子命令，`GoalSpec` 也没有 prerequisite 字段。因此 B1 必须由外层监督器读取 §3 依赖并只提交一个 ready 节点；禁止预创建全部 Goal，禁止调用待删除的 compatibility parser。

## 1. 外部输入与已合入工作

| 输入 | 当前状态 | Owner / 关系 |
|---|---|---|
| DeepSeek 缓存计划（C0–C7） | `accepted` | C-plan 继续拥有 provider/cache/usage 字段；X9c/X12 只消费 typed 输出 |
| 14 条历史 feature/fix/test 分支 | `already_merged` 到 `origin/dev` | X0 仅保存对账证据，不再 rebase 或重复合并 |
| 6 份计划/证据文档 | `accepted`（PR #162，merge `95a5f8046f64eccd0ea6f22b0a82f06465e72419`） | Goal DAG 已启动 |

### 1.1 单执行器下的 owner 边界

X、R 轨道可由同一监督器串行承担，但 owner 边界仍约束定义权：provider/cache/usage 字段只由 C-plan 定义；robot 域契约只由 R-plan 定义；X 轨道只能消费和投影。没有显式启用并行执行器时，所谓“可并行”仅表示依赖允许，不代表同时启动多个 Goal。

C 轨道保留 C0–C7 编号，不进入本文件 DAG；R 轨道保留 R0–R8，在 X13 汇合。

### 1.2 C 轨道字段冻结 gate（已满足）

`deepseek-cache-and-message-optimization-plan.md:48-60` 已将 C0–C7 全部标记为 `accepted`；阶段 STATUS 块位于 `:373,454,541,647,737,821,915`。因此：

```text
C_FIELD_FREEZE = satisfied
X9c / X12      = 不再因 C 轨道阻塞
```

后续若 C 轨道变更已冻结字段名、单位或 `unknown` 语义，必须按 C-plan §6.4 记录 wire migration；X 轨道不得在 projection 层静默兼容。

## 2. 依赖关系（唯一真相 = §3 表格的“依赖”列）

执行顺序一律读取 §3 的依赖列。轨道关系：

- **核心 X 轨道**：从 X0 开始，按 §3 的精确依赖推进；X14 属于核心完成条件。
- **X11 corpus**：只依赖 X0；单执行器时作为其他节点阻塞后的候选，不同时运行 workspace build。
- **R 轨道**：依赖见 §3.4；外部 Policy/Bridge/MuJoCo 是否可用由阶段 preflight 判定，不能用“不做物理实机”直接阻塞 R8。
- **C 轨道**：C0–C7 已关闭，只保留 owner 边界。

`scripts/cargo-agent.sh:9,36` 使用 `flock` 共享构建锁；仍禁止并发运行 `executive` 或 workspace-wide build。只有明确启用多个执行器时才允许并行节点，且写入范围必须不重叠。

## 3. 节点清单（唯一 ID；旧 ID 映射见 §5）

以下拆分已经满足 §8.2：跨 presentation/application 的原节点被拆开，原 X9 的 9 个验收 ID 被拆为 X9a/X9b/X9c。每个节点独立分支、独立 PR、可独立回滚。

| ID | 内容 | 依赖 | 写入范围 | 交付/删除 | 退出证据（验收 ID 见 §4） |
|---|---|---|---|---|---|
| **X0** | 基线/在途对账与架构冻结 | B0/B1 | `docs/plans/*`、`config/architecture/*` | 保存 §3.1 已合入分支证据；刷新两个 `frozen_commit`；确认 status ledger/approval/control-plane | 两个 `frozen_commit`=`PLAN_BASELINE_SHA`；B0/B1 证据齐全 |
| **X1** | 契约迁移矩阵 + 增量门禁 + ID 台账 | X0 | `config/architecture/*`、checker/fixtures、`fabric` 契约 | existing/extend/project/v2/new/delete 矩阵；禁用边；创建无 live-status 列的 `acceptance-ids.tsv`；3 个计数器 | A-DEP-001/002/003 + negative fixtures |
| **X2** | `TaskKindArg` 迁移 | X1 | `crates/interact/src/tui/cli.rs:97-107` → CommandSpec 权威位置；`crates/aletheon/src/main.rs` | 纯迁移，不删文件 | §3.3 计数 1→0；窄 check 通过 |
| **X3a** | Command/Intent application contract | X2 | Fabric command contract、Executive command use case | 定义/迁移 `CommandSpec`、`ClientIntent`；Executive `CommandDispatcher` 成为唯一 handler | A-ENTRY-001 + application contract tests |
| **X3b** | 用户入口 adapter 收敛 | X3a | 顶层 CLI、Interact slash/line adapter、Gateway adapter、completion assets | 所有入口只转换 `ClientIntent`；help/completion/parser 来自同一 spec | U-CLI-001/002 |
| **X3c** | compatibility parser 删除 | X3b | `crates/interact/src/tui/cli.rs`、`crates/interact/src/lib.rs` | 删除 `Args::parse()`、旧 handler/re-export、line-mode 手写 dispatch | A-ENTRY-002/003 + U-CLI-004 + 计数 0 |
| **X4a** | daemon lifecycle/doctor application | X3c | Executive launcher、readiness、doctor use cases | install-mode resolve/activate/spawn、lock、readiness、timeout、version negotiation | U-BOOT-003 + lifecycle/竞态测试 |
| **X4b** | Interact ensure-running adapter | X4a | `crates/interact` host/TUI | system install/user-local/dev foreground 连接闭环；stale socket 和结构化诊断 | U-BOOT-001/002 |
| **X4c** | run/exec wire 与 application | X3a, X4a | Fabric wire、Executive exec host | versioned JSONL、退出码、stdin、idempotency、backpressure | U-CLI-003 + wire/host tests |
| **X4d** | run/resume/completion CLI adapter | X3b, X4b, X4c | 顶层 CLI、completion assets | `run`/`resume`/`completion` 仅适配 canonical contract；补 canonical Goal control surface 或明确保留 B1 external supervisor | help snapshot、PTY、completion、Goal control-plane evidence |
| **X5a** | Session 权威映射表（doc-only） | X1, X3a | `docs/plans/`、`config/architecture/wire-surfaces.tsv` | 现有 `SessionAppendStore`/`EventSpine`/`EventProjection`/`SessionProjection` → 唯一 SessionAuthority | reviewed keep/extend/absorb/delete 映射；无同名新权威 |
| **X5b** | daemon-owned Session/Task/Activity projection | X5a | Executive projection、Fabric read protocol | versioned snapshot/event read model，可确定性 replay | A-SESSION-001/002 |
| **X5c** | Interact projection reducer | X5b | Interact reducer | TUI 状态只由 snapshot+events 重建，不拥有 writer | A-SESSION-003 + reducer replay/out-of-order tests |
| **X6a** | TUI task console | X5c | `crates/interact/src/tui/` | Conversation/Task/Changes、Activity timeline、keyboard hints，删除 transcript 重复状态 | U-TUI-001..007 |
| **X6b** | Secure input | X6a | Interact editor/completion/history/attachment | Action Palette、`@`、受治理 `!`、history/draft、IME/paste/ANSI 防护 | U-INPUT-001..006 |
| **X7** | Capability/transaction coverage | X1, X5b | Executive application ports、Corpus/domain adapters | invocation/receipt envelope；mutation coverage；事务/补偿/拒绝 | A-CAP-001..005 + A-TURN-001 |
| **X8a** | Checkpoint/validation projection | X5b, X7 | Fabric/Executive checkpoint、validation projection | 复用现有 `TurnCheckpoint`；生成 coverage-aware projection | U-CHK-003/004 + projection integrity tests |
| **X8b** | Diff/Accept/Repair/Rollback/Rewind UI | X6a, X8a | Interact TUI | diff/picker；full/best-effort/non-rollbackable 如实展示 | U-CHK-001/002/005/006 |
| **X8c** | Host review/settlement application | X7, X8a | Executive validation/acceptance | `ReviewFinding`；Host 唯一 settlement writer；失败回到 repair | U-VERIFY-001..004 + A-TURN-002 |
| **X8d** | review CLI/TUI projection | X8b, X8c | 顶层 CLI、Interact review view | `aletheon review` 和 TUI 只投影 findings/settlement | CLI/TUI snapshot + terminal receipt consistency |
| **X9a** | Session recovery 与 generation fencing | X1, X5b | daemon recovery、session/checkpoint persistence | restart/replay、迟到 generation 拒绝、principal 隔离、patch 幂等 | U-RESUME-001/002/004/005/006 |
| **X9b** | External runtime/child reconciliation | X7, X8c, X9a | runtime adapters、Executive child supervision | orphan reconciliation；terminal receipt；权限/预算/workspace fencing | U-RESUME-003 + A-AGENT-001/002/003 |
| **X9c** | Provider machine authority 与显式 fallback | X1, X5b, §1.2 C-freeze | machine core、provider runtime、daemon recovery | 消费 C-plan typed facts；处置 system-scope core orphan；新增 `SILENT_FALLBACKS` 计数器 | machine concurrency/cooldown tests；orphan process evidence；计数器=0 |
| **X10** | 剩余 compatibility ledger 删除 | X3c, X9a, X9b, X9c | `compatibility-debt.tsv` 指定路径、README | 删除到期 re-export/writer/type alias；README 只描述唯一生产路径 | A-DELETE-001/002 + architecture check |
| **X11** | U-corpus fixtures 8→20 | X0 | `tests/coding/acceptance/` | 补 12 个 versioned fixture + rubric，不建平行目录 | 20 个 fixture 可运行 |
| **X12** | 安装态工程验收 | X8d, X10, X11, §1.2 C-freeze | acceptance fixtures/docs；缺陷使用 §8.7 `XF-*` | 系统部署；所有运行中 Aletheon executable SHA 一致；restart 稳定；official socket 真实 LLM；20-task ≥16 | U-INST-001/002/003 + 架构计划 §9.5 |
| **X13** | Robot 统一主链验收 | X12, R7 `accepted`, R6 ≥ `code_complete` | Hardware/robot projection、Interact robot view | robot 复用 Task/Activity/Receipt；MuJoCo；Safety denial=blocked | A-ROBOT-001/002 + U-ROBOT-001/002/003；不含物理实机 |
| **X14** | Memory 权威与 GBrain 边界 | X5b | Mnemosyne、Dasein、Executive memory ports | taxonomy/authority；governed intake；provenance/scope/freshness；GBrain supplemental | A-MEM-001/002/003 |

### 3.1 X0 的在途对账清单（2026-08-05 实测）

下列 14 条历史分支在 X0 以 `git merge-base --is-ancestor <branch> origin/dev` 复核，处置统一为 `already_merged`；记录的 SHA 是本地保留分支尖端，不得 rebase、cherry-pick 或重复合入：

```text
auro/feat/20260730-coding-evaluation-kernel       8ce12a827c08387c75613c1b986b58d94aa5f974  already_merged
auro/feat/20260731-governed-review-service        555b5656e65864f056a04ebba28427fff3e6328e  already_merged
auro/feat/20260801-unified-memory-design          8a1fb438144923f14bf901b0ed0dbf0d8e4cbf19  already_merged
auro/feat/20260803-inference-cache-contract       ba841df1f73e8722636be5ea1a29c81dbcd6442f  already_merged
auro/feat/20260804-robot-embodiment-runtime       9a93b630a0d709c9ab2d27c441ef3ad10b7a0c06  already_merged
auro/feature/20260804-deepseek-cache              758af8a808029ca09ee5d3d950b707d5aa8ecee7  already_merged
auro/fix/20260804-gbrain-memory-pipeline          87f8fec322f227d590857ca0d80c0efa9d4f4650  already_merged
auro/fix/20260804-main-release-gates              b203d69e38067d35882581ff4112a32394a5d65b  already_merged
auro/fix/20260804-msrv-contract                   e21d1dc3769e45a6cb1473889f90a8cdc97641d3  already_merged
auro/fix/20260804-strict-quality-evolution        26eae99caab42abb6f8db0df8c0e64821476d9e0  already_merged
auro/fix/20260804-strict-quality-final            74acfbf673f1b742afa249405ca79956fd84e7ae  already_merged
auro/fix/20260804-strict-quality-tail             cbbd89210c7e0666829b9141d47e379c25bfa332  already_merged
auro/test/20260731-plan-acceptance-cleanup        cd0e68e8f19e2b3502ee0d664ccaae01cbcd5113  already_merged
auro/test/20260801-remaining-production-gates     c1c030ba9ddb490401bc0ab8d334a2489d9d01e6  already_merged
```

B0 开工时工作树仅包含 §0.1 的计划/证据文档；该事实已由 PR #162 的提交范围保存。后续节点仍以 `git status --porcelain` 现算，发现无归属改动必须先判定 owner，不能混入节点分支。

### 3.2 新增架构计数器的归属

| 计数器 | 归属节点 | 期望值 | 说明 |
|---|---|---|---|
| `PRODUCTION_CLI_PARSERS` | X1 定义 / X3c 达标 | 1 | 统计被生产入口调用的 Clap parser |
| `SESSION_APPEND_WRITERS` | X1 定义 / X5b 达标 | 1 | 统计非授权 session append writer |
| `FORBIDDEN_DEPENDENCY_EDGES` | X1 定义并达标 | 0 | 复用禁用边检测 |
| `SILENT_FALLBACKS` | X9c 定义并达标 | 0 | runtime fallback event 已存在后落地 |

`metrics.env` 只允许下降；X0 把两个 frozen commit 推进到 `PLAN_BASELINE_SHA`，后续节点只在 reviewed ratchet 更新中推进。

### 3.3 “生产调用计数”的唯一计算命令

```bash
grep -rn "interact::cli" --include=*.rs crates/ | grep -v "^crates/interact/"
```

基线输出 1 行：`crates/aletheon/src/main.rs:15:use interact::cli::TaskKindArg;`。X2 结束时为 0；X3c 删除 compatibility parser/re-export。

### 3.4 Robot 轨道（依赖与状态上限的唯一调度表）

| ID | 内容 | 依赖 | 状态上限 / 外部 gate |
|---|---|---|---|
| R0 | 基线冻结 + 证据清单 | B0 | 当前 `in_progress`；补 bridge version、proto digest、scene version 并入库后才可 `accepted` |
| R1 | Robot/Policy typed config | R0 | `accepted` |
| R2 | 启动 capability gate | R1 | 无外部服务时 `code_complete` |
| R3 | Perception/FrameRef 数据链 | R1 | `accepted` |
| R4 | 真实 VLA Policy gateway | R2, R3 | 无真实 gateway 时 `code_complete` |
| R5 | typed failure + bounded replan | R4 | `accepted` |
| R6 | 实机安全 capability manifest | R2 | 无真实设备证据时 `code_complete` |
| R7 | Episode artifact/报告闭环 | R1 | `accepted` |
| R8 | 安装态真实 Policy+Bridge+MuJoCo E2E | R4, R5, R6, R7 | 缺 Policy/Bridge/MuJoCo 任一项才 `externally_blocked`；物理实机不是本阶段前置 |

X13 不含物理实机，因此 R6 达 `code_complete` 即满足 X13 前置；R7 必须 `accepted`。R8 的 MuJoCo 安装态关闭与物理实机/HIL gate 分开判定。

## 4. 验收 ID → 证据落点

### 4.1 命名规则（唯一机械判定手段）

每个验收 ID 必须至少对应一个测试函数，函数名前缀 = ID 小写、`-`换`_`：

```text
A-ENTRY-002  ->  fn a_entry_002_<描述>()
U-BOOT-001   ->  fn u_boot_001_<描述>()
```

判定命令：

```bash
grep -rn "fn a_entry_002" --include=*.rs crates/ tests/
```

基线实测：全仓库排除 `docs/plans/` 后，**所有验收 ID 命中 0**。也就是说这些 ID 目前只是散文，
没有任何机械约束力——这是本次修改要解决的首要问题。

### 4.2 机器可读验收台账

X1 创建 `config/architecture/acceptance-ids.tsv`，列：

```text
# id\tnode\tkind\ttarget
A-ENTRY-002\tX3c\ttest\tcrates/aletheon/tests/cli_contract.rs
U-INST-002\tX12\tmanual-evidence\tdocs/testing/installed-acceptance.md
```

- TSV 是**静态注册表**，只拥有 ID、节点、证据类型和证据落点，不保存 live status。
- live status 的唯一权威是 `docs/plans/execution-status.md`，避免双状态写入。
- `kind`：`test` / `command` / `manual-evidence`；manual evidence 只允许安装态/外部服务类 ID。
- X1 之后由 architecture checker 验证每个 `kind=test` 的函数前缀存在。

### 4.3 ID → 节点归属（完整覆盖）

| 节点 | 拥有的验收 ID |
|---|---|
| X1 | A-DEP-001, A-DEP-002, A-DEP-003 |
| X2 | 无；§3.3 计数命令 |
| X3a | A-ENTRY-001 |
| X3b | U-CLI-001, U-CLI-002 |
| X3c | A-ENTRY-002, A-ENTRY-003, U-CLI-004 |
| X4a | U-BOOT-003 |
| X4b | U-BOOT-001, U-BOOT-002 |
| X4c | U-CLI-003 |
| X4d | 无；CLI/PTY/completion/Goal control-plane evidence |
| X5a | 无；reviewed authority mapping |
| X5b | A-SESSION-001, A-SESSION-002 |
| X5c | A-SESSION-003 |
| X6a | U-TUI-001 … U-TUI-007 |
| X6b | U-INPUT-001 … U-INPUT-006 |
| X7 | A-CAP-001 … A-CAP-005, A-TURN-001 |
| X8a | U-CHK-003, U-CHK-004 |
| X8b | U-CHK-001, U-CHK-002, U-CHK-005, U-CHK-006 |
| X8c | U-VERIFY-001 … U-VERIFY-004, A-TURN-002 |
| X8d | 无；CLI/TUI projection 与 terminal receipt consistency |
| X9a | U-RESUME-001, U-RESUME-002, U-RESUME-004, U-RESUME-005, U-RESUME-006 |
| X9b | U-RESUME-003, A-AGENT-001, A-AGENT-002, A-AGENT-003 |
| X9c | 无；provider machine-boundary tests、orphan evidence、`SILENT_FALLBACKS=0` |
| X10 | A-DELETE-001, A-DELETE-002 |
| X11 | 无；20 个 fixture 可运行 |
| X12 | U-INST-001, U-INST-002, U-INST-003 |
| X13 | A-ROBOT-001, A-ROBOT-002, U-ROBOT-001, U-ROBOT-002, U-ROBOT-003 |
| X14 | A-MEM-001, A-MEM-002, A-MEM-003 |

源计划中每个 ID 的语义保持不变；本表只重新分配 owner。无 ID 的节点仍必须满足 §3 的明确退出证据。

## 5. 旧 ID 映射（三份源计划的历史 PR 表 → 本文件）

| 本文件 | 架构计划历史分组 | UX 计划历史分组 |
|---|---|---|
| X0 | P0 | U0/P0 |
| X1 | P1 | — |
| X2 | P2a | U1b-1 |
| X3a | P2b contract/application | U1a command contract |
| X3b | P2b adapters | U1a user adapters |
| X3c | P2b deletion | U1b-2 |
| X4a | runtime governance | U1c application half |
| X4b | — | U1c Interact half |
| X4c | command application | U1d wire/application half |
| X4d | command adapter | U1d CLI half |
| X5a | P3 mapping | — |
| X5b | P3 daemon projection | U2 projection producer |
| X5c | P3 UI consumer | U2 projection reducer |
| X6a | P5 | U2 TUI task console |
| X6b | — | U3 secure input |
| X7 | P4 | U4 capability coverage |
| X8a | P6 projection | U4 checkpoint projection |
| X8b | P6 UI | U4 rewind UI |
| X8c | P6 settlement | U5 application |
| X8d | P6 presentation | U5 CLI/TUI |
| X9a | P7 recovery | U6 session recovery |
| X9b | P7 runtime | U6 child reconciliation |
| X9c | P7 provider governance | U6 runtime facts |
| X10 | P8 remaining ledger | — |
| X11 | — | U-corpus |
| X12 | P9 | U7 |
| X13 | P10 | U8 |
| X14 | 原历史表漏项 | — |

## 6. 关键裁决（消除歧义）

1. `TaskKindArg` 在 X2 先迁移；用户 adapter 在 X3b 收敛；compatibility parser 只在 X3c 删除。X10 不再处理该 parser。
2. 唯一 SessionAuthority 是现有 `EventSpine`/`SessionAppendStore`/`SessionProjection` 的收敛，不新造同义权威；X5a reviewed 后才做 X5b/X5c。
3. Host 是唯一 settlement writer；模型文本、child 自报或 monitor PASS 均不能直接写 `accepted`。
4. C0–C7 已 `accepted`，X9c/X12 的 C-freeze gate 已满足；字段定义权仍归 C-plan。
5. R8 和 X13 均不要求物理实机。R8 只有在真实 Policy、Bridge 或 MuJoCo 缺失时才 `externally_blocked`。
6. system-scope core 孤儿进程由 X9c 处置；X12 动态枚举并核对所有运行中的 Aletheon daemon executable，包括 machine core、user daemon 和 Memory Agent（若运行）。
7. X11 依赖允许提前执行，但单执行器不并发启动；不得以构建锁存在为由并发跑 `executive`/workspace build。
8. Extension package 是 Skill/Hook/MCP 生命周期的唯一安装权威；产品默认不新增 `skill`/`hook`/`mcp` 三套平行顶层生命周期命令。
9. compatibility parser 是零生产调用的内部债务，可在 X3c 直接删除；真正公开、仍有调用者的命令才适用 release-cycle deprecation。

## 7. 状态词表与完成定义

### 7.1 状态词表（唯一定义处）

| 状态 | 含义 | 可否关闭节点 |
|---|---|---|
| `not_started` | 未开工 | 否 |
| `in_progress` | 已开工，退出证据未全部通过 | 否 |
| `code_complete` | 仓库内代码/契约/确定性测试完成，但存在外部未验证项 | 否 |
| `externally_blocked` | 缺外部条件（真实服务/设备/官方 key）无法推进 | 否，但**不计为失败** |
| `accepted` | 退出证据全部通过并留下证据 | **是** |

只有 `accepted` 关闭节点。禁止自创第六种状态。禁止把 `#[ignore]` 测试的存在写成通过。

### 7.2 完成定义

**核心完成（X 轨道）** = X0–X12 与 X14 全部 `accepted`，且：

1. 生产 CLI parser / Command handler / Session writer / Turn engine / Capability side-effect path
   各只有 1 个权威（由 §3.2 计数器机械证明）。
2. 所有修改声明 mutation coverage；`full` 可自动 Accept/Repair/Rollback，
   `best_effort/non_rollbackable` 走显式风险+审批或拒绝。
3. UI/CLI/Gateway 只是 adapter/projection，不拥有业务状态。
4. External runtime 是受监督 child，不经 Host 验证不能推进父任务。
5. 20 个真实工程任务 ≥16 成功，evidence/receipt 完整，monitor 与 runtime evidence 无冲突。
6. X12 provenance gate 通过：release/installed binary 与所有运行中 Aletheon daemon executable SHA 一致、systemd restart 稳定、official socket 真实 LLM 请求。
7. 兼容层（parser/re-export/双轨 writer）已删除，deprecation ledger 无过期项。
8. Mnemosyne/Dasein/Agora/Cognit/Executive 的 owner 边界可由 X14 的测试证明。

**外部阻塞时的诚实结论**：若 X12 的 official socket 真实请求项为 `externally_blocked`，
整体状态只能写 **`code_complete`（代码关闭，生产验收待执行）**，不得写 completed，
也不得用 fixture 冒充真实验收。此条优先于 §7.2 的其他表述。

**Robot 域扩展完成**独立于核心完成：X13 `accepted` 关闭 robot 用户主链；R8 关闭安装态 Policy/Bridge/MuJoCo，物理实机/HIL 另行判定。

## 8. Goal 无人值守运行规则

### 8.1 当前能力边界与监督器职责

当前 `GoalSpec` 只有 intent/desired state/constraints/acceptance/budget，没有 prerequisite；Goal worker 只按 state/wait reason 选取 Goal，并将同一 `original_intent` 交给 attempt。因此：

```text
Night Supervisor
  -> 读取 execution-status.md
  -> 选择依赖全部 accepted 的一个节点
  -> 创建并运行一个 Goal
  -> 等待 authoritative terminal snapshot / durable receipt
  -> 执行节点退出证据
  -> commit -> PR dev -> CI -> repair -> merge
  -> 验证 merge SHA 并更新 execution-status.md
  -> 再选择下一个 ready 节点
```

- 任一时刻最多一个 active Goal；禁止预创建整个 DAG。
- Goal 成功文本不是节点成功；只有监督器观察 terminal snapshot、退出证据和 merge 结果后才可写 `accepted`。
- `EXECUTION_CONTROL_PLANE` 必须在 B1 固定；不得在运行中从 external supervisor 切到 compatibility parser。
- 每个 Goal 只对应一个 X/R/XF 节点；禁止跨节点修改或把多个节点合成一次 settlement。

### 8.2 节点体量规则

单节点触及 >12 个源文件、跨 presentation/application、或拥有 >8 个验收 ID 时，必须先 doc-only 拆分。§3 已拆分已知违规节点；实际 file inventory 若再次越界，监督器停止实现并先修正文档。X6a 的 7 个 ID 接近上限，开工前仍须确认实际文件数。

### 8.3 Budget、deadline 与重试

当前代码默认 `max_attempts=10`、无 cost/deadline；`goal.create` 还会留下空 constraints/acceptance。无人值守执行不得使用这些默认值：

- 每节点 Goal 必须显式填充 §3 的 scope、constraints、acceptance criteria。
- `max_attempts=3`：首次执行 + 最多 2 次退出证据修复。
- 每节点原则上设置 deadline、input/output token 上限和 cost cap；owner 可以在 `execution-status.md` 显式写 `unbounded_by_owner` 豁免这些额度。额度豁免不改变 `max_attempts=3`、依赖、证据、PR/CI 和连续 blocked 停止规则。
- 编译/lint/格式迭代不单独计为 acceptance repair，但不得绕过 Goal 总 deadline。

| 情形 | 处理 |
|---|---|
| 退出证据首次失败 | 缩小到具体断言，记录 FAILURES，允许 repair 1 |
| repair 1 仍失败 | 更新假设与证据，允许 repair 2 |
| repair 2 仍失败 | 保持 `in_progress`，写 `BLOCKER`，停止该节点 |
| 缺真实服务/设备/key | `externally_blocked`，不伪造 fixture 通过 |
| 连续 2 个节点 blocked | 停止整晚循环，等待 owner |
| 计划缺陷或外部未登记 dev 提交 | 先 doc-only reconciliation，再恢复 |

### 8.4 Git/PR/CI/merge 闭环

每个节点执行固定流程：

1. `git fetch origin dev`，确认 base 与 status ledger；从 `origin/dev` 创建 `auro/<type>/<date>-<node>-<slug>`。
2. 只修改 §3 的写入范围；先跑最窄验证，Rust 一律经 `bash scripts/cargo-agent.sh`。
3. 检查 staged diff；按仓库 commit 格式提交完整 subject/body，禁止 subject-only 非平凡提交。
4. push 分支并创建目标为 `dev` 的 PR；记录 PR URL/number。
5. 监控所有 required checks；失败只在同一节点分支修复，不绕过、不取消测试。
6. checks 通过后按 `AUTO_MERGE_AUTHORIZATION` 决定：获授权才执行 merge；否则标 `externally_blocked` 等 owner。
7. 确认 PR 状态为 merged，并验证 merge commit 已进入 `origin/dev`。
8. 更新该节点 STATUS；下一节点必须从新的 `origin/dev` 开始。

当前 remote 是 GitHub 且已安装 `gh`；B1 必须先验证认证。标准命令骨架：

```bash
git push -u origin HEAD
gh pr create --base dev --head "$(git branch --show-current)" --title "<subject>" --body-file <summary-file>
gh pr checks --watch <pr-number>
# 仅 AUTO_MERGE_AUTHORIZATION=approved 时：
gh pr merge --merge <pr-number>
gh pr view <pr-number> --json state,mergeCommit
git fetch origin dev
git merge-base --is-ancestor <merge-sha> origin/dev
```

禁止 force-push 已供 review 的分支、禁止删除用户分支、禁止把不相关改动带入节点 PR。没有 GitHub 写权限或 merge 授权时标 `externally_blocked`，不能把“本地 commit 完成”写成节点关闭。

### 8.5 命名约定

| 项 | 约定 | 示例 |
|---|---|---|
| 分支 | `auro/<type>/<date>-<node>-<slug>` | `auro/refactor/20260805-x3c-compat-parser` |
| commit 前缀 | `<type>(<node>)` | `refactor(x3c): delete compatibility parser` |
| robot 节点 | 前缀使用 `r` | `feat(r1): add robot typed config` |
| 动态缺陷 | `fix(xf-NNN)` | `fix(xf-001): reconcile installed daemon` |
| doc-only reconciliation | `docs(plan)` | `docs(plan): reconcile external dev changes` |

一个节点可包含多个有意义 stage commit，但必须同一节点、可整体回滚；不得为了“一节点一提交”压扁不便审查的大改动。

### 8.6 STATUS 唯一落点

`docs/plans/execution-status.md` 是 live status 唯一权威；`acceptance-ids.tsv` 不保存 status。每个节点独立段落使用：

```text
STATUS: not_started | in_progress | code_complete | externally_blocked | accepted
NODE: X3c
BASE: <origin/dev commit>
BRANCH: <branch>
PR: <url/number/status>
GOAL_ID: <id>
BUDGET: attempts/input/output/cost/deadline
SCOPE: 修改文件；明确未修改的外部仓库
CONTRACT: config/proto/public type/state transition 的变化
VALIDATION: 完整命令、退出码、关键负向断言
EVIDENCE: 验收 ID → 测试函数或证据文件
RUNTIME EVIDENCE: in-process/direct live/installed
ROLLBACK: commit/兼容/迁移说明
FAILURES: 每次 acceptance failure 与修复结果
BLOCKER: 无则 none
OPEN ITEMS: 外部依赖，不得伪装完成
MERGE_SHA: accepted 后填写
```

### 8.7 X12 动态缺陷节点

X12 发现产品缺陷时创建 `XF-001` 起的节点，并先登记到本文件和 status ledger：

- 依赖 = 发现它的节点；原节点保持 `in_progress`。
- scope、acceptance、budget、rollback 与普通节点相同。
- XF `accepted` 并合入后，原节点从最新 `origin/dev` 重跑全部退出证据。
- 不得把多个无关缺陷合并成一个 “misc fixes” PR。

| ID | 发现节点 | 缺陷与根因 | 写入范围 | 退出证据 |
|---|---|---|---|---|
| XF-001 | X12 | coding harness 仍读取废弃的 flat `stop`/metrics，因而把 canonical `ExecEventEnvelope` v1 terminal 误判为缺失 | `tests/coding/harness/`、runner/receipt tests、status ledger | canonical v1 terminal fake-client coverage；provider terminal fail-closed；static harness suite |
| XF-002 | X12 | 安装态 auto sandbox 同时丢失普通 host toolchain identity 与 configured-profile 下的 workspace writable bind：runner 在 `--clearenv` 后只恢复 Git 字段，policy mount path 又跳过但未安装 WorkspacePolicy roots | `crates/corpus/src/security/runner.rs`、`crates/corpus/src/security/sandbox/bubblewrap.rs`、`config/aletheon.user.service`、status ledger | 环境 allowlist/secret exclusion tests；configured-profile workspace bind/process tests；Corpus clippy；system unit/deployment verification；安装态 auto sandbox 内解析并执行 toolchain，同时 workspace 外保持只读 |
| XF-003 | X12 | benchmark 在模型执行后、hidden acceptance 前采集 Git scope，但 Rust 的正常 `cargo` 验证会把未忽略的 `target/` 误算为模型越界修改 | `tests/coding/harness/run.py`、runner tests、status ledger | fixture private Git exclude 覆盖；source change 仍可见；`target/` 不进入 changed paths/diff；static harness suite |
| XF-004 | X12 | real-model coding run repeatedly emitted the documented `*** Update File`/bare `@@` patch form, while Platform structured-patch parser required Aletheon-only unprefixed headers and `>>>` fences | `crates/platform/src/structured_patch.rs`、parser/application tests、status ledger | canonical fenced format remains valid；common model update/add form parses and applies；path traversal remains rejected；Platform tests/clippy |
| XF-005 | X12 | sandbox correctly keeps host `/tmp` read-only, but rustc requires a writable temporary directory and command execution provided no private scratch mount/TMPDIR | `crates/corpus/src/security/runner.rs`、`crates/corpus/src/security/sandbox/bubblewrap.rs`、tests、status ledger | per-call private scratch lifecycle；TMPDIR/TMP/TEMP point only to that bind；workspace/outside-write invariants remain；installed auto-sandbox `cargo test` |
| XF-006 | X12 | receipt classifier rejected an expected authoritative `blocked`/`budget_exhausted` outcome whenever the denial path incremented `tool_errors`, even though the error is causal evidence for the expected non-success terminal | `tests/coding/harness/receipt.py`、runner/replay tests、status ledger | matched non-success terminal tolerates its tool error；verified/failed/provider errors remain fail-closed；static harness suite |
| XF-007 | X12 | installed corpus runs were repeatedly invalidated by a concurrent checkout deploying and restarting `/usr/bin/aletheon` mid-suite; provenance was sampled per receipt but no runtime-generation lease prevented mutation | `scripts/aletheon.sh`、`tests/coding/harness/suite.py`、deployment/suite tests、status ledger | deploy holds exclusive lease for build→verify；installed suite holds shared lease for full catalog；diagnostic binaries do not lock；lock override is testable |
| XF-008 | X12 | the integration architecture gate failed because `crates/corpus/src/security/runner.rs` exceeded its governed hotspot budget after the sandbox fixes; the excess was inline test code rather than production admission logic | `crates/corpus/src/security/runner.rs`、`crates/corpus/src/security/runner/tests.rs`、architecture gate、status ledger | production runner returns below its frozen line budget；private test access and test names remain unchanged；Runner tests and Corpus clippy pass |
| XF-009 | X12 | an installed code-change probe produced the correct patch and successful validation but timed out because the cognitive loop required `AcceptChange` evidence even though `change_accept` is intentionally Host-only and absent from production model tools | Cognit change-transaction completion observation/tests、status ledger | model loop closes after version-bound diff review and validation；Host-only acceptance remains outside model tools；premature/stale evidence still fails closed；installed probe reaches a terminal receipt |
| XF-010 | X12 | the installed monitor reached the official user daemon but falsely reported its systemd unit inactive because MCP processes lacked `XDG_RUNTIME_DIR`/session-bus variables and monitor subprocesses inherited that incomplete environment | monitor health/diagnose systemd probes and tests、status ledger | derive the user-manager environment from the authoritative user socket UID；never replace explicit caller values；health and TUI preflight report the real unit state |

## 9. 节点状态台账（指针）

实时状态见 `docs/plans/execution-status.md`。初始状态：

| ID | 初始状态 |
|---|---|
| B0/B1 | `not_started`；完成并记录 owner approval 后才能启动 X0 |
| X0–X14 及所有子节点 | `not_started` |
| R0 | `in_progress`；证据缺 bridge version/proto digest/scene version，且尚未入库 |
| R1–R8 | `not_started` |
| C0–C7 | `accepted` |
