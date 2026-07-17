# G7 可执行 Spec：记忆检索与凭证安全强化

> 对应研究文档 `../09-memory-search-and-credential-safety.md`。优先级 P3（独立，仅吸收策略）。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：在 Mnemosyne 现有 FTS5 检索之上补三项：(a) **authority/scope/sensitivity 前置过滤**（检索前或 candidate 物化前，不靠 prompt 层事后删除）；(b) **可选 vector KNN + FTS 降级**（embedding 不可用退 FTS-only，单一装配参数）；(c) **endpoint-scoped embedding 凭证**（精确 origin 匹配，fail-closed，Debug 隐藏）。

**非目标**：
- 不替换 Mnemosyne authority/scope/retention/promotion 模型。
- 不让 vector score 越过 authority/scope/sensitivity 过滤。
- 不引入无凭证约束的远程 embedding。
- 第一版可先只做 (a) 前置过滤 + FTS fallback 测试；vector/MMR 为后续。

## 2. 当前代码锚点（已验证 @ commit bec15695）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| Mnemosyne 导出 | `crates/mnemosyne/src/lib.rs:23-60` | MemoryAuthority/Kind/Scope/Sensitivity/Status、ScopeAncestry、RecallItem/Request/Set、MemoryProjectionLimits |
| `RecallRequest` | `crates/mnemosyne/src/service.rs:67-74` | session/query/max_items/max_content_bytes/current_at/include_historical |
| `RecallSet` | 同上 `:125-129` | `items: Vec<RecallItem>`、`degraded_sources: Vec<String>` |
| `RecallItem` | 同上 `:114-121` | content/metadata/temporal_state/authority/scope |
| `MemoryService::recall` | 同上 `:283`；impl `:596-625` | 调 recall_memory.search_in_session + fact_store.search_facts + episodic/core |
| FTS5 | `crates/mnemosyne/src/impl/recall_memory.rs:43-65` | porter tokenizer + BM25；LIKE fallback(119-122) |
| vector store trait | `crates/mnemosyne/src/impl/vector_store.rs:54-73` | `search(query: &[f32], top_k, filter)->Vec<ScoredEntry>`；Qdrant(75-122) **未接入 recall** |
| `AgentMemoryContext` | `crates/mnemosyne/src/agent_scope.rs:33-82` | process/agent/task id + agent_scope/task_scope + parent_projection_receipt |
| `ScopeAncestry::allows` | `crates/mnemosyne/src/model/scope.rs:43-52` | 校验 principal/session/goal/agent/task 对 scope |
| `MemoryProjectionLimits` | `crates/mnemosyne/src/projection.rs:22-26` | max_items=8/max_total_bytes=16K/max_item_bytes=2K |
| 凭证/endpoint | `vector_store.rs:20-26` | `VectorStoreConfig` 有 qdrant_url/lance_path 但**无 api_key/auth** |
| 过滤点 | `crates/mnemosyne/src/recall/local.rs:11-191` | authority/scope **检索后**才赋值；**无前置过滤** |
| projection 校验 | `projection.rs:99-100` | 检索后剔除无效项，非检索前 |

**核心缺口**：(1) authority/scope/sensitivity **检索后**才应用（应前置）；(2) vector 基建存在但未接入且无 fallback 编排；(3) 无 endpoint-scoped 凭证。

## 3. 权威归属决策（doc10 §6 八问）

1. **owner**：Mnemosyne 拥有全部；G7 只在其内部补前置过滤 + fallback + 凭证约束。
2. **scope**：检索按 `AgentMemoryContext` 的 verified scope ancestry；不让 child 用查询文本猜 parent/global scope。
3. **crash 恢复**：检索无持久态；索引 stale 用 lexical + last valid vector snapshot。
4. **fail 模式**：embedding endpoint 不可信 → 不发凭证、FTS-only；FTS DB 故障 → 按现有降级不伪造成功；redirect/origin 变化 → 撤销凭证附着。
5. **上限**：复用 `MemoryProjectionLimits`；candidate 数上限。
6. **兼容**：flag 关闭 → 走当前 FTS-only recall（等价）。
7. **进 event spine**：degraded health、凭证拒绝可记录（不含 key）。
8. **许可证**：重新实现检索流水线与凭证约束，不复制 Grok `xai-grok-memory/backend.rs`。

## 4. 类型定义

### 4.1 前置过滤 + 检索流水线 — `crates/mnemosyne/src/recall/pipeline.rs`（新文件）

```rust
//! 检索流水线：安全过滤尽量前置，再 candidate retrieval，再确定性 merge/rank。
use crate::model::scope::ScopeAncestry;
use crate::{MemoryAuthority, MemorySensitivity};

/// 前置过滤谓词：在 SQL/candidate 物化前就限定可见 scope/authority/sensitivity。
#[derive(Debug, Clone)]
pub struct RecallPreFilter {
    /// 来自 verified AgentMemoryContext 的 scope ancestry。
    pub ancestry: ScopeAncestry,
    /// 允许的最高敏感度。
    pub max_sensitivity: MemorySensitivity,
    /// 允许的 authority 集合。
    pub allowed_authorities: Vec<MemoryAuthority>,
}

impl RecallPreFilter {
    /// 转成检索层可下推的条件（FTS WHERE 子句 / vector filter）。
    /// 关键：不能先取出跨用户内容再事后删。
    pub fn to_scope_predicate(&self) -> ScopePredicate { unimplemented!() }
}

/// 可下推到 FTS 与 vector 两条检索路径的统一谓词。
#[derive(Debug, Clone)]
pub struct ScopePredicate {
    pub scope_keys: Vec<String>,
    pub max_sensitivity_ord: u8,
}
```

### 4.2 混合检索 + 降级 — 同文件

```rust
/// 单一装配参数：避免不同构建路径悄悄退化（对齐 Grok backend 思路，
/// 但归 Mnemosyne composition root）。
#[derive(Debug, Clone)]
pub struct RecallSearchParams {
    pub fts_enabled: bool,
    pub vector_enabled: bool,
    pub top_k: usize,
    pub use_mmr: bool,
}

/// 检索健康降级信号（进 RecallSet.degraded_sources）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DegradedSource {
    NoEmbeddingConfig,
    EmbeddingEndpointUntrusted,
    EmbeddingTimeout,
    VectorIndexStale,
    FtsDbError,
}

/// 混合检索：pre-filter → (FTS lexical + optional vector KNN) → 确定性 merge/rank
/// → projection 上限。embedding 不可用则 FTS-only 并记 degraded。
pub async fn hybrid_recall(
    pre: &RecallPreFilter,
    params: &RecallSearchParams,
    /* fts backend, optional vector backend, request */
) -> (Vec<crate::RecallItem>, Vec<DegradedSource>) { unimplemented!() }
```

### 4.3 Endpoint-scoped 凭证 — `crates/mnemosyne/src/recall/credential.rs`（新文件）

```rust
/// embedding provider 凭证授权，绑定精确 origin。
#[derive(Clone)]
pub struct EmbeddingCredentialGrant {
    pub principal: fabric::PrincipalId,
    /// 规范化 scheme+host+port+base path。必须精确相等才附着凭证。
    pub approved_base_url: String,
    pub provider_id: String,
    /// 只允许 embedding 操作。
    pub operation: EmbeddingOperation,
    pub expiry_unix: u64,
    pub rotation_generation: u32,
    /// 凭证句柄——Debug 隐藏。
    secret: SecretHandle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingOperation { EmbeddingOnly }

/// 隐藏凭证的 Debug（绝不打印 key）。
#[derive(Clone)]
pub struct SecretHandle(String);
impl std::fmt::Debug for SecretHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretHandle(***)")
    }
}

impl EmbeddingCredentialGrant {
    /// fail-closed：请求 base url 必须与 approved 精确相等（对齐 Grok approved_for）。
    /// hostname 后缀宽松匹配、redirect 后续用均拒绝。
    pub fn approved_for(&self, request_base_url: &str, now_unix: u64) -> bool {
        now_unix < self.expiry_unix
            && normalize_url(request_base_url) == self.approved_base_url
    }
}

fn normalize_url(u: &str) -> String { unimplemented!() /* scheme+host+port+base path 规范化 */ }
```

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 新增 | `crates/mnemosyne/src/recall/pipeline.rs` | 前置过滤 + 混合检索 + 降级 |
| 新增 | `crates/mnemosyne/src/recall/credential.rs` | endpoint-scoped 凭证 |
| 修改 | `crates/mnemosyne/src/service.rs:596-625` | `recall` 改走 pipeline：先 pre-filter 再检索 |
| 修改 | `crates/mnemosyne/src/impl/recall_memory.rs:43-65` | FTS 查询下推 scope predicate（前置过滤） |
| 修改 | `crates/mnemosyne/src/impl/vector_store.rs:20-26` | `VectorStoreConfig` 接凭证（经 grant，不存 key） |
| 修改 | `crates/mnemosyne/src/lib.rs` | 导出新类型 |
| 修改 | feature flag | `grok_hardening.memory_hybrid` 默认关（关时 FTS-only 当前行为） |

## 6. 任务分解（TDD）

**阶段 A：前置过滤（最高价值，独立于 vector）**
- T1. `RecallPreFilter` + `to_scope_predicate`。单测：ancestry 生成正确 scope keys。
- T2. FTS 查询下推 predicate（recall_memory.rs）：SQL WHERE 含 scope/sensitivity 条件。单测：跨 scope 记录**不进** candidate。
- T3. `service.rs::recall` 先 pre-filter：断言过滤在检索前（不是事后剔除）。集成测试。
- T4. child 用 verified `AgentMemoryContext` 生成 ancestry；不能用查询文本猜 parent/global scope。单测。

**阶段 B：降级编排（FTS fallback）**
- T5. `RecallSearchParams` 单一装配参数。`hybrid_recall` 无 embedding config → FTS-only + `degraded=[NoEmbeddingConfig]`。单测。
- T6. FTS DB 故障 → 按现有降级、`degraded=[FtsDbError]`、不伪造成功。单测。

**阶段 C：endpoint-scoped 凭证**
- T7. `EmbeddingCredentialGrant::approved_for`：精确 base url 相等才 true；hostname 后缀宽松 → false；过期 → false。单测。
- T8. `SecretHandle` Debug 隐藏——断言 `format!("{:?}")` 不含 key。单测。
- T9. redirect/origin 变化 → 不携带凭证到新 origin。单测。
- T10. endpoint 不可信 → 不发凭证、FTS-only、`degraded=[EmbeddingEndpointUntrusted]`。单测。

**阶段 D：vector 接入（可选，flag 后）**
- T11. `hybrid_recall` 接 vector：pre-filter 同样约束 vector filter（score 不越 scope）。单测：vector rank 不返回 scope 不允许记录。
- T12. 确定性 merge/rank：同输入 + 同索引版本，merge 结果可重复。属性测试。
- T13. vector index stale → lexical + last valid snapshot，`degraded=[VectorIndexStale]`。单测。

**阶段 E：收尾**
- T14. clippy/fmt；更新 §2 漂移；标注 flag 灰度。

## 7. 兼容与迁移

- **flag 关闭**：`recall` 走当前 FTS-only（等价）。
- **前置过滤可先行**：阶段 A 不依赖 vector，可独立交付并默认启用（安全增强，非行为变更——本就该过滤）。
- **vector 可选**：阶段 D 依赖 embedding 配置；无配置恒 FTS-only。
- **不替换 Mnemosyne authority**：所有新逻辑服从现有 authority/scope/sensitivity/promotion。

## 8. 测试计划（映射研究文档 ../09 §7 验收方向）

| 验收方向 | 测试 |
|---|---|
| embedding 完全不可用时 lexical recall 仍工作 | T5 |
| 不可信 endpoint 永不收到 session 凭证 | T10 |
| redirect/origin 变化撤销凭证附着 | T9 |
| vector rank 不返回 scope 不允许记录 | T11 |
| result 保留 authority/provenance/temporal state | T3（RecallItem 字段保留） |
| 同输入同索引版本 merge/rank 可重复 | T12 |

## 9. 可观测性

- 事件：`memory.recall.degraded`（degraded_sources）、`memory.credential.rejected`（provider、原因，**不含 key**）。
- 指标：`recall_fts_only_total`、`recall_vector_used_total`、`embedding_credential_rejected_total`、`recall_prefilter_excluded_total`。
- 日志：凭证 origin 不匹配以 `warn`（脱敏 url）。

## 10. 许可证

重新实现检索流水线、降级策略与 endpoint-scoped 凭证语义（参考 Grok `approved_for` fail-closed 思路），不复制 Grok `xai-grok-memory/backend.rs` 源码。无 NOTICE 变更。
