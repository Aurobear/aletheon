# 记忆检索与凭证安全

## 1. Grok 可借鉴的两点

Grok memory backend 将 FTS5 keyword search 与可选 vector KNN 合并；embedding 不可用时降级到 FTS-only（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-memory/src/backend.rs:1-10`）。它把 embedding 凭证绑定到可信、可解析 endpoint，不匹配时 fail closed，并在 Debug 中隐藏 credential handle（同文件 `:21-89`）。

这些是检索工程与凭证安全策略，不是 memory authority 模型。

## 2. Aletheon 不能被替换的部分

Mnemosyne 已提供：

- agent scope/vault/draft
- authority、kind、provenance、scope、sensitivity、status、temporal state
- bounded projection
- retention/forgetting
- local + supplemental recall
- promotion receipt

这些公共边界见 `crates/mnemosyne/src/lib.rs:23-60`。Grok 的 memory backend 不应替换它们，也不应让 vector score 越过 authority/scope/sensitivity 过滤。

## 3. 推荐检索流水线

```text
RecallRequest
  -> authority/scope/sensitivity pre-filter
  -> candidate retrieval
       +-- FTS lexical
       `-- optional vector KNN
  -> deterministic merge/rank
       +-- source weights
       +-- recency/temporal state
       +-- authority weight
       `-- diversity/MMR (optional)
  -> projection byte/item limits
  -> provenance-preserving RecallSet
```

关键顺序：安全过滤应尽可能在检索前或 candidate materialization 前完成；不能先把跨用户内容取出再依赖 prompt 层删除。

## 4. 降级策略

| 故障 | 行为 |
|---|---|
| 无 embedding 配置 | FTS-only |
| embedding endpoint 不可信 | 不发送凭证，FTS-only |
| endpoint timeout/rate limit | 记录 degraded health，FTS-only |
| vector index stale | 使用 lexical + last valid vector snapshot |
| FTS DB 故障 | supplemental/local 其他源按现有策略降级，不伪造成功 |

Grok 的 backend 把全部 session search 配置聚合为单一 params，避免不同构建路径悄悄退化（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-memory/src/backend.rs:92-110`）；Aletheon 可借鉴“单一装配参数”，但配置应属于 Mnemosyne composition root。

## 5. Endpoint-scoped credential

建议任何远程 embedding/search provider 的 credential grant 绑定：

- principal/account
- exact normalized scheme + host + port + base path policy
- provider id
- allowed operation（embedding only）
- expiry/rotation generation
- audit source

禁止：

- 只按 hostname 后缀宽松匹配。
- redirect 后继续携带 credential 到新 origin。
- 在 Debug、event、memory provenance 中保存 key。
- child Agent 自行更换 endpoint 后继承 parent credential。

Grok 的 `approved_for` 要求请求 base URL 与已批准 URL 相等（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-memory/src/backend.rs:84-89`），这一 fail-closed 思路值得保留。

## 6. 与多 Agent memory isolation 的关系

Aletheon child memory 已有独立 vault/promotion 边界，导出位于 `crates/mnemosyne/src/lib.rs:23-31`。混合检索必须先使用 verified `AgentMemoryContext` 生成 scope ancestry，再搜索；不得让 child 用查询文本猜测 parent/global scope。

检索结果进入 workspace/conscious context 前，继续通过 `MemoryProjectionLimits` 等有界投影；该 API 当前导出于 `crates/mnemosyne/src/lib.rs:46-49`。

## 7. 验收方向

- embedding 完全不可用时 lexical recall 仍工作。
- 不可信 endpoint 永远收不到 session credential。
- redirect/origin 变化会撤销 credential attachment。
- vector rank 不会返回 scope 不允许的记录。
- result 保留 authority/provenance/temporal state。
- 同一输入和索引版本的 merge/rank 可重复。

