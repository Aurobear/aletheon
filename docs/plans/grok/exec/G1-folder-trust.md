# G1 可执行 Spec：Folder Trust 与多用户工作区信任

> 对应研究文档 `../02-folder-trust-and-multi-user-workspaces.md`。优先级 P0。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：允许从任意非 `/` 目录启动并读写普通文件（已具备），同时对**仓库提供的可执行配置**（repo-local hooks / MCP server 命令 / plugin 可执行 / `.envrc` / LSP 命令 / agent 命令型扩展）做信任门控——未信任则不加载或询问；trust receipt 绑定 principal + workspace identity + config digest，多用户互不污染。

**非目标**：
- 不改 `WorkspacePolicy` 的写权限模型（信任 ≠ 写权限）。
- 不做完全拒绝工作区（未信任 → restricted，不是 reject）。
- 本期不实现 receipt 的持久 revocation UI；只做存储 + 过期 + digest 失效。
- 不做实际的 hooks/MCP/plugin 加载器（那些是各自 feature）；本期只产出 `WorkspaceTrustDecision` 供加载器查询。

## 2. 当前代码锚点（已验证 @ commit bec15695，含未提交修改）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| `WorkspaceSelection` | `crates/fabric/src/types/local_authority.rs:187-190` | `{ cwd: Option<PathBuf>, add_dirs: Vec<PathBuf> }` |
| `WorkspaceSelection::resolve` | 同上 `:197-199` | `resolve(self, process_cwd: &Path) -> Result<WorkspacePolicy, WorkspaceResolveError>` |
| 隐式 `/` 拒绝 | 同上 `:214-216` | `cwd == "/" && !explicitly_selected -> Err(ImplicitFilesystemRoot)` |
| canonicalize | 同上 `:213` | `std::fs::canonicalize()`（TOCTOU 防护基础已在） |
| `WorkspacePolicy` | 同上 `:71-76` | `{ cwd, writable_roots, protected_paths }`，构造后不可变 |
| `ProtectedPathPolicy` | 同上 `:125-153` | `{ credential_paths }`，writable_roots 内仍只读 |
| `CapabilityExecutionContext` | `crates/executive/src/service/governed_capability.rs:22-37` | 可信字段：`principal:PrincipalId`(26)、`connection_id`(27)、`thread_id`(28)、`turn_id`(29)、`workspace:WorkspacePolicy`(30)、`sandbox`(33)、`cancel`(34) |
| **现有 trust 层** | — | **无**（grep 无结果，全新实现） |
| 事件发布 | `crates/fabric/src/ipc/bus/communication_bus.rs:164-179` | `publish_event_v2(schema: SchemaId, source, payload) -> Result<()>` |
| `PrincipalId` | `crates/fabric/src/types/admission.rs:32-39` | `pub struct PrincipalId(pub String)`，格式 `local-uid:{uid}` |
| `TurnRequest.context` | `crates/fabric/src/include/turn.rs:12` | `PrincipalContext` 携带 `workspace: WorkspacePolicy` |
| 消费点 | `crates/executive/src/service/context_assembler.rs:69` | `request.context.workspace.cwd()` |

## 3. 权威归属决策（doc10 §6 八问）

1. **权威 owner**：Fabric 定义 `WorkspaceTrustDecision`/`TrustReceipt` 类型与纯函数 `decide()`；Executive 拥有 trust store（持久化）与 evaluate 编排；Interact 负责交互 prompt。
2. **scope**：receipt 按 `(principal_id, canonical_workspace_identity)` 主键持久化。
3. **crash 恢复**：trust store 是幂等 upsert；重启后按 digest 重新校验，digest 变则旧 receipt 不自动授权。
4. **fail 模式**：headless/daemon 无法交互 → 默认 **不信任**（restricted）；trust store 读失败 → fail closed（当作未信任）。
5. **上限**：discovered config digest 输入有界（只 hash 已知配置文件路径集合，不递归全仓）。
6. **兼容**：flag 关闭时 `evaluate()` 恒返回 `Trusted`（等价当前无门控行为）。
7. **进 event spine**：新增 trust 决策事件经 `publish_event_v2` 发布（decision、blocked_sources、granting client）。
8. **许可证**：重新实现 `decide` 纯函数语义，不复制 Grok `folder_trust.rs`。

## 4. 类型定义

### 4.1 新增 Fabric 类型 — `crates/fabric/src/types/workspace_trust.rs`（新文件）

```rust
//! 工作区信任决策。约束"仓库提供的可执行配置"是否加载，不改变 cwd 是否可用。
//! 纯类型 + 纯决策函数；持久化与交互在 Executive/Interact。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use crate::types::admission::PrincipalId;

/// 受信任门控的"仓库提供的执行入口"类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ExecutableConfigSource {
    RepoHooks,
    RepoMcpServer,
    RepoPlugin,
    EnvrcLoader,
    LspServer,
    RepoAgentCommand,
}

/// 客户端交互能力：决定"未记录信任"时是询问还是默认不信任。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMode {
    /// 可弹窗询问（交互式 TUI/ACP）。
    Interactive,
    /// 无法交互（headless/daemon/CI）→ 默认不信任。
    Headless,
}

/// 工作区的规范身份，防路径别名/软链绕过。canonical_path 来自
/// WorkspacePolicy 已 canonicalize 的 cwd。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIdentity {
    pub canonical_path: PathBuf,
    /// 可用时的 repo 远端指纹（git remote url 规范化后 hash），否则 None。
    pub repo_fingerprint: Option<String>,
}

/// 发现的可执行配置摘要。key=类别，value=该类别下所有配置文件内容的稳定 digest。
/// 配置变化 → digest 变 → 旧 receipt 失效。发现过程只读、不解释执行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredConfigDigest(pub BTreeMap<ExecutableConfigSource, String>);

/// 信任凭证。持久化单元。绑定 principal + workspace + digest + 授权范围。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustReceipt {
    pub principal_id: PrincipalId,
    pub workspace: WorkspaceIdentity,
    pub digest: DiscoveredConfigDigest,
    /// 只授权部分类别（如允许 hooks 不允许 MCP）。
    pub granted: Vec<ExecutableConfigSource>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    /// 过期时间戳；None = 不过期。
    pub expires_at_unix: Option<u64>,
    /// 授权来源客户端/连接标识，审计用。
    pub granting_client: String,
}

/// 决策输入（全部可信，非模型可伪造）。
#[derive(Debug, Clone)]
pub struct TrustEvaluationInput {
    pub principal_id: PrincipalId,
    pub workspace: WorkspaceIdentity,
    pub discovered: DiscoveredConfigDigest,
    pub client_mode: ClientMode,
    /// feature flag：关闭时决策恒 Trusted。
    pub feature_enabled: bool,
    /// 该 (principal, workspace) 现存 receipt（若有）。
    pub existing_receipt: Option<TrustReceipt>,
    /// 配置里标记的"宽根"（不可记录持久信任的目录，如 $HOME 根）。
    pub is_broad_unrecordable_root: bool,
    pub now_unix: u64,
}

/// 决策结果。限定三态（对齐 Grok 的 Trusted/Untrusted/Prompt）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceTrustDecision {
    /// 加载 receipt 授权的类别。
    Trusted { granted: Vec<ExecutableConfigSource> },
    /// 不加载任何 repo-local 可执行配置（普通文件仍可用）。
    Restricted { blocked: Vec<ExecutableConfigSource> },
    /// 需要交互确认。findings = 发现的可执行配置类别。
    PromptRequired { findings: Vec<ExecutableConfigSource> },
}
```

### 4.2 纯决策函数 — 同文件

```rust
/// 纯函数：给定输入，产出决策。无 I/O、无副作用，可完全单测。
/// 优先级（对齐 Grok folder_trust §2）：
///   1. feature off            -> Trusted(全部)  （等价当前行为）
///   2. 无发现的可执行配置       -> Trusted(空)    （没什么可门控）
///   3. 宽根不可记录            -> Restricted     （不在 $HOME 根记录持久信任）
///   4. 有效 receipt 且 digest 未变 -> Trusted(receipt.granted)
///   5. Headless               -> Restricted     （无法询问，默认不信任）
///   6. 其余（Interactive）      -> PromptRequired
pub fn decide(input: &TrustEvaluationInput) -> WorkspaceTrustDecision {
    use WorkspaceTrustDecision::*;

    // 1. feature off：等价当前无门控。
    if !input.feature_enabled {
        return Trusted { granted: all_sources() };
    }

    let found: Vec<ExecutableConfigSource> =
        input.discovered.0.keys().copied().collect();

    // 2. 无可执行配置：无需门控。
    if found.is_empty() {
        return Trusted { granted: vec![] };
    }

    // 3. 宽根：不记录持久信任，直接 restricted（避免把 $HOME 标为信任）。
    if input.is_broad_unrecordable_root {
        return Restricted { blocked: found };
    }

    // 4. 有效且 digest 未变的 receipt。
    if let Some(r) = &input.existing_receipt {
        let not_expired = r.expires_at_unix.map_or(true, |e| input.now_unix < e);
        let digest_match = r.digest == input.discovered;
        let principal_match = r.principal_id == input.principal_id;
        let workspace_match = r.workspace == input.workspace;
        if not_expired && digest_match && principal_match && workspace_match {
            return Trusted { granted: r.granted.clone() };
        }
        // digest 变 / 过期 → 落到下面重新决策（不自动继承）。
    }

    // 5. Headless：无法交互 → 默认不信任。
    if input.client_mode == ClientMode::Headless {
        return Restricted { blocked: found };
    }

    // 6. Interactive：询问。
    PromptRequired { findings: found }
}

fn all_sources() -> Vec<ExecutableConfigSource> {
    use ExecutableConfigSource::*;
    vec![RepoHooks, RepoMcpServer, RepoPlugin, EnvrcLoader, LspServer, RepoAgentCommand]
}
```

### 4.3 Executive 端口 — `crates/executive/src/service/workspace_trust.rs`（新文件）

```rust
use async_trait::async_trait;
use fabric::types::workspace_trust::*;

/// trust receipt 持久化端口。生产实现落 sqlite/文件；测试用内存。
#[async_trait]
pub trait TrustStore: Send + Sync {
    async fn get(&self, principal: &PrincipalId, ws: &WorkspaceIdentity)
        -> Option<TrustReceipt>;
    /// 幂等 upsert，主键 (principal, workspace)。
    async fn put(&self, receipt: TrustReceipt);
}

/// 只读发现器：扫描 workspace 的已知配置文件路径，产出 digest。不执行、不解释。
#[async_trait]
pub trait ConfigDiscoverer: Send + Sync {
    async fn discover(&self, workspace_cwd: &std::path::Path)
        -> DiscoveredConfigDigest;
}

/// 编排：发现 → 取 receipt → decide → 发事件。交互 prompt 由上层（Interact）处理
/// PromptRequired 后回调 record_grant()。
pub struct WorkspaceTrustResolver {
    store: std::sync::Arc<dyn TrustStore>,
    discoverer: std::sync::Arc<dyn ConfigDiscoverer>,
    feature_enabled: bool,
}
```

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 新增 | `crates/fabric/src/types/workspace_trust.rs` | 类型 + `decide()` 纯函数 |
| 修改 | `crates/fabric/src/types/mod.rs` | 导出 `workspace_trust` |
| 新增 | `crates/executive/src/service/workspace_trust.rs` | `TrustStore`/`ConfigDiscoverer`/`WorkspaceTrustResolver` |
| 修改 | `crates/executive/src/service/mod.rs` | 导出 |
| 新增 | `crates/executive/src/impl/.../trust_store_file.rs` | 文件/sqlite `TrustStore` 生产实现 |
| 新增 | `crates/executive/src/impl/.../config_discoverer.rs` | 只读发现器（扫 `.grok`/`.aletheon` hooks/mcp/... 路径） |
| 修改 | `crates/executive/src/impl/daemon/bootstrap/request.rs:77-87` | 装配 resolver 到 RequestHandler |
| 新增 | schema 注册 | trust 决策事件的 `SchemaId` |
| 修改 | feature flag config | `grok_hardening.folder_trust` 默认关 |

## 6. 任务分解（TDD，2-5 分钟粒度）

**阶段 A：Fabric 类型 + 决策函数（纯，最高价值先做）**
- T1. 新建 `workspace_trust.rs`，写全部类型。`cargo check -p fabric`。
- T2. 写 `decide()`。单测 case 1：`feature_enabled=false` → `Trusted(all)`。
- T3. 单测 case 2：`discovered` 空 → `Trusted([])`。
- T4. 单测 case 3：宽根 → `Restricted(found)`。
- T5. 单测 case 4a：有效 receipt digest 匹配 → `Trusted(receipt.granted)`。
- T6. 单测 case 4b：digest 变 → 不继承（Interactive→Prompt，Headless→Restricted）。
- T7. 单测 case 5：Headless + 有配置 + 无 receipt → `Restricted`。
- T8. 单测 case 6：Interactive + 有配置 + 无 receipt → `PromptRequired`。
- T9. 单测：**多用户隔离** — Alice 的 receipt 对 Bob（principal 不匹配）不授权。
- T10. 单测：过期 receipt 不授权。
- T11. `mod.rs` 导出。`cargo test -p fabric`。

**阶段 B：Executive 端口 + 内存实现**
- T12. 写 `TrustStore`/`ConfigDiscoverer` trait + `WorkspaceTrustResolver`。`cargo check -p executive`。
- T13. 内存 `TrustStore` 测试替身 + upsert 幂等单测（同 key 两次 put 只留最新）。
- T14. `WorkspaceTrustResolver::evaluate()`：串起 discover→get→decide。集成测试（内存替身）：Interactive 首次 → Prompt；record_grant 后二次 → Trusted。

**阶段 C：只读发现器**
- T15. `ConfigDiscoverer` 生产实现：枚举已知路径（`.grok/hooks`、`.grok/mcp.json`、`.envrc`、agent 定义），读内容算稳定 digest（排序 + sha256）。**只读断言测试**：发现器不写、不 exec。
- T16. 测试：改动配置文件内容 → digest 变。

**阶段 D：持久化 + 事件**
- T17. 文件/sqlite `TrustStore`（复用现有 sqlite journal crate 若有）。crash 恢复测试：写入后重读一致。
- T18. 注册 trust 决策 `SchemaId`；`evaluate()` 内经 `publish_event_v2` 发 decision/blocked/granting_client 事件。事件断言测试。

**阶段 E：装配（flag 后）**
- T19. `bootstrap/request.rs` 装配 resolver；flag 关时 resolver 直返 `Trusted(all)`（旁路），回归测试证明等价当前行为。
- T20. 端到端：任意非 `/` 目录启动成功；不可信仓库普通文件可读写、repo-local 配置不加载（用 decision 断言，加载器 stub）。

**阶段 F：收尾**
- T21. `cargo clippy`/`fmt`。更新 §2 漂移。标注 flag 灰度。

## 7. 兼容与迁移

- **flag 关闭**：`decide()` case 1 直返 `Trusted(all)`；resolver 旁路。完全等价当前无门控。
- **加载器解耦**：本期不改 hooks/MCP/plugin 实际加载；它们后续接入时查询 `WorkspaceTrustDecision`。本期交付"决策 + 存储 + 事件"。
- **headless 默认安全**：daemon 场景 `ClientMode::Headless` → restricted，不等待不可用的交互输入。

## 8. 测试计划（映射研究文档 ../02 §7 验收方向）

| 验收方向 | 测试 |
|---|---|
| 任意非 `/` 目录启动成功 | T20（复用现有 resolve 隐式 `/` 拒绝，不回归） |
| 不可信仓库普通文件可用、repo 配置不运行 | T20 |
| headless 默认 restricted | T7, T19 |
| 两 principal 信任互不污染 | T9 |
| digest 变旧 receipt 失效 | T6, T16 |
| 决策/blocked/授权者进审计事件 | T18 |

属性测试：`decide()` 对任意输入必返回三态之一，且 `feature_enabled=false` 恒 `Trusted(all)`。

## 9. 可观测性

- 新事件（经 `publish_event_v2`）：`workspace.trust.decided`（principal、workspace canonical path、decision、blocked_sources、granting_client）。
- 指标：`trust_prompt_required_total`、`trust_restricted_headless_total`、`trust_digest_invalidated_total`。
- 日志：digest 失效 / 宽根拒绝以 `info` 记录带 principal + workspace。

## 10. 许可证

重新实现 `decide()` 语义与 receipt 模型，不复制 Grok `xai-grok-workspace/src/folder_trust.rs`。无 NOTICE 变更。多用户 receipt 模型是 Aletheon 原创扩展（Grok 只有本机单布尔 trust store）。
