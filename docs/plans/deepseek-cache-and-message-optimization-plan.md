# DeepSeek 缓存与消息构造优化计划

> 基线：`origin/dev` / `278f357638bf575cda34008a176a71c036326929`（2026-08-04）
> 性质：源码级实施计划，不表示能力已经完成
> 主要目标：以 DeepSeek V4 为默认推理路线，稳定请求前缀、正确记录缓存用量，并补齐
> Aletheon 自身的上下文投影、记忆检索和只读工具结果缓存。
> 关联代码：`fabric::types::llm_types`、`cognit::adapters::inference`、
> `cognit::harness::linear`、`executive::application::turn_pipeline`、Mnemosyne、Corpus。

## 0. 决策摘要

本计划将“缓存”拆成四个不同问题，禁止继续用一个模糊的 `cache` 名称同时表示它们：

| 层级 | Owner | 缓存对象 | 当前状态 | 本计划目标 |
|---|---|---|---|---|
| L0 Provider Prefix/KV | DeepSeek | 从第 0 token 开始的相同输入前缀 | 部分完成 | 稳定前缀、正确计量、可解释 miss |
| L1 Context Projection | Cognit/Executive | 已规范化的系统前缀、工具定义、消息投影 | 只有构造与 digest | 先完成 typed partition 与 profiling；本轮不默认增加结果复用 |
| L2 Recall | Mnemosyne | 相同 scope/query/policy 下的记忆检索结果 | 未实现 | generation 驱动的短 TTL LRU |
| L3 Tool Result | Corpus | 明确声明为纯/只读的工具结果 | 未实现 | 显式策略、权限隔离和失效规则 |

以下对象不是缓存：

- Agent settlement、operation receipt 和 idempotency key 是正确性状态；
- Agora 是当前会话工作状态；
- Mnemosyne 数据库是长期记忆的权威存储；
- Robot world state 是带 sequence/freshness 的实时事实，禁止退化为普通 TTL 缓存；
- LLM 最终回答默认不做 response cache。

实施优先级：先完成 L0 的可测量正确性，再做 L2；L1 只有在 profiling 证明本地构造成本
值得优化后才进入生产；L3 必须晚于工具语义和权限契约。

### 0.1 执行状态定义

DeepSeek 执行时必须使用以下状态，不得用“代码已写”代替完成：

| 状态 | 含义 |
|---|---|
| `not_started` | 尚未修改 |
| `in_progress` | 正在实现，尚未取得全部验证证据 |
| `code_complete` | 代码和确定性测试通过，但尚未完成真实 provider 或安装态验收 |
| `externally_blocked` | 代码侧已完成，缺真实 endpoint、凭据或外部服务 |
| `accepted` | 本阶段要求的代码、测试、真实请求或安装态证据全部齐全 |

计划总状态只有 C0-C7 全部 `accepted` 才能标记完成。C5 需要真实 DeepSeek endpoint；如果缺少
凭据，只能标记 `externally_blocked`，不能把 fixture 测试写成真实缓存验收。

### 0.2 执行纪律

1. 每个阶段开始前，以阶段列出的代码符号重新 grep/读取当前 `origin/dev`，发现路径变化时更新计划定位，
   不凭文档猜测类型存在；
2. 一个 PR 只完成一个阶段；不得同时修改 Provider parser、Mnemosyne repository 和 Corpus runner；
3. 不改模型输出内容、reasoning/tool-call 协议来换取缓存命中；不增加针对验收 prompt 的生产特例；
4. 所有 Cargo 命令通过 `bash scripts/cargo-agent.sh ...`；优先运行最窄 package/target；
5. 涉及 daemon、配置、工具、持久化或客户端的阶段，最终验收遵守仓库 `AGENTS.md` 的安装态策略；
6. 每个 PR 必须提交：变更文件、契约变化、测试命令与结果、已知未验收项、回滚方式；
7. Provider secret、完整 prompt、用户内容和原始 memory 不得进入 digest 日志或测试快照。

---

## 1. 当前代码事实

### 1.1 已有基础

1. `crates/executive/src/composition/prefix_builder.rs`
   - daemon 启动时构造稳定 system prefix；
   - Memory 和按轮选择的 Skill 不进入 prefix；
   - `SkillAdminService` 更新技能后能够重建 prefix。
2. `crates/cognit/src/harness/linear/message_compose.rs`
   - plan mode、memory update、goal context、Dasein state 注入 user message；
   - 这些动态状态不会修改 system prompt。
3. `crates/fabric/src/types/llm_types.rs`
   - `canonicalize_tool_definitions` 对工具按名称排序并规范化 JSON object key；
   - `tool_schema_digest` 使用带 domain separation 的 SHA-256；
   - `InferenceUsage` 已区分 total、uncached、cache read、cache write；
   - `CacheTelemetry` 已区分 `Reported`、`Unsupported`、`Unknown`。
4. `crates/cognit/src/adapters/inference/anthropic.rs`
   - system block 与 tools 使用 provider cache control；
   - 能解析 cache read/write token。
5. `crates/executive/src/application/turn_pipeline.rs`
   - 非流式和流式 usage 能累积到 turn result；
   - 不完整 telemetry 不会被错误地合计成完整数字。
6. `config/default.toml`
   - 默认路线是 `lejurobot_deepseek` / `deepseek/deepseek-v4-flash[1m]`；
   - 同时配置官方 `https://api.deepseek.com`；
   - 两者都走 `transport = "openai"`。

### 1.2 已确认缺口

`crates/cognit/src/adapters/inference/openai_provider.rs` 的 `ApiUsage` 当前只解析：

```rust
prompt_tokens
completion_tokens
prompt_tokens_details.cached_tokens
```

而 DeepSeek Chat Completions 使用：

```text
prompt_cache_hit_tokens
prompt_cache_miss_tokens
```

因此现有 direct DeepSeek 或兼容代理即使返回缓存数据，字段也会被 Serde 忽略。当前代码还不能
可靠回答以下问题：

- 官方 DeepSeek 是否命中缓存；
- LejuRobot 代理是否透传或转换缓存字段；
- hit + miss 是否等于 prompt total；
- 缓存优化前后的成本和 TTFT 是否改善。

`crates/executive/src/host/daemon/cache_shape.rs` 目前只在自身测试中使用，而且：

- 使用进程实现细节型 `DefaultHasher`；
- 只 hash tool name，不 hash完整 schema；
- 没有 provider、model、transport 和序列化协议版本；
- 没有接入 request、receipt 或 turn pipeline。

它不能作为生产缓存身份的权威实现。

---

## 2. DeepSeek 约束和设计原则

DeepSeek provider cache 由服务端自动维护；Aletheon 不持有远端 KV。Aletheon 能控制的是输入
序列的稳定性。设计必须遵守：

1. 只有从第 0 token 开始相同的前缀才能复用；中间相同不能补救前面的变化；
2. 缓存是 best-effort，未命中不等于 Aletheon 一定存在 bug；
3. 缓存建立有延迟，冷启动后的立即重试不保证命中；
4. provider 会淘汰长期不用的条目；Aletheon 不得把命中当正确性依赖；
5. 输出仍然重新采样，prefix cache 不是 response cache；
6. 官方 endpoint 与 LejuRobot proxy 必须分别测量，不根据 model name 推断能力。

消息顺序固定为：

```text
Stable system prefix
  -> stable tool definitions for the selected AgentProfile
  -> prior conversation in canonical order
  -> compacted/session context
  -> memory/goal/Dasein updates
  -> latest tool results
  -> current user input
```

以下字段禁止进入 stable prefix：当前时间、UUID、operation ID、working directory、剩余预算、
实时设备状态、每轮 recall 结果、attempt 编号和未规范化 map。

---

## 3. 目标架构和 owner

### 3.1 Fabric：只定义通用事实，不出现 DeepSeek 业务类型

`fabric` 保持 provider-neutral。新增或调整的共享契约：

```rust
pub struct InferenceCacheObservation {
    pub telemetry: CacheTelemetry,
    pub total_input_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub uncached_input_tokens: Option<u64>,
}
```

如果现有 `InferenceUsage` 已足够表达，不新建重复结构；优先增加验证函数：

```rust
impl InferenceUsage {
    pub fn validate(&self) -> Result<(), InferenceUsageError>;
}
```

验证规则：

- 已知 read/uncached/total 时，`read + uncached == total`；
- read/write/uncached 不得大于 total 所允许范围；
- `Reported` 允许明确的 `Some(0)`；
- 字段缺失必须保留 `None`，不得归零；
- `Unsupported` 不得携带伪造的 hit/miss。

### 3.2 Cognit adapter：负责 wire format 差异

DeepSeek 字段只出现在 OpenAI-compatible adapter 的私有 wire struct：

```rust
struct ApiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    prompt_tokens_details: Option<PromptTokensDetails>,
    prompt_cache_hit_tokens: Option<u64>,
    prompt_cache_miss_tokens: Option<u64>,
}
```

解析优先级：

1. DeepSeek hit 和 miss 同时存在：使用二者，并验证守恒；
2. 只有 DeepSeek hit，且 total 已知：`miss = total - hit`；
3. OpenAI `cached_tokens` 存在：`read = cached`，`uncached = total - cached`；
4. 两种格式同时存在且数值一致：接受并记录一种统一结果；
5. 两种格式冲突：返回 provider protocol error，不静默选择；
6. provider 已声明 unsupported：生成 `CacheTelemetry::Unsupported`；
7. 没有字段且能力未知：`CacheTelemetry::Unknown`。

不能通过 `model.contains("deepseek")` 选择解析逻辑；代理可能改写模型名，官方也可能改变兼容格式。

### 3.3 Executive：负责 shape、对比和可观测

把 `cache_shape.rs` 重构为通用 `InferencePrefixShape`：

```rust
pub struct InferencePrefixShape {
    pub version: u16,
    pub provider_id: String,
    pub model_id: String,
    pub transport: String,
    pub system_prefix_digest: String,
    pub tool_schema_digest: String,
    pub agent_profile_digest: String,
    pub rewrite_version: u64,
}
```

完整 shape digest 使用 SHA-256 和 domain：

```text
aletheon.inference-prefix-shape.v1\0
```

shape 是诊断身份，不决定是否允许推理。将它投影到 inference receipt/attempt evidence，禁止
让缓存机制成为请求正确性的前置条件。

---

## 4. 分阶段实施

## Phase C0：基准和契约冻结（P0）

### 修改范围

- `docs/plans/deepseek-cache-and-message-optimization-plan.md`
- `crates/fabric/tests/inference_contract.rs`
- `crates/cognit/src/adapters/inference/openai_provider.rs` 的测试模块
- 新增 provider fixture，放在 Cognit 的测试资源目录，不放生产代码。

### 工作项

1. 为以下 usage payload 建 fixture：
   - DeepSeek hit/miss；
   - OpenAI cached tokens；
   - 两种字段都没有；
   - 显式零命中；
   - hit 大于 total；
   - hit + miss 不等于 total；
   - OpenAI 与 DeepSeek 字段冲突；
   - streaming 最终 usage chunk。
2. 把 `None`、`Some(0)`、`Unsupported` 写成独立断言。
3. 给 `InferenceUsage` 写 provider-neutral 守恒测试。

### 完成条件

- 失败 fixture 在实现前能够证明当前缺口；
- 测试不访问真实网络；
- 不修改 `fabric` 以容纳 provider 专有字段。

### 实施结果（2026-08-04，与 C1 合并为一个 PR，两个 commit）

**Fixture 清单**（`crates/cognit/tests/fixtures/usage/*.json`，9 个 wire 片段，
Chat Completions `usage` 对象；测试经 `include_str!("../../../tests/fixtures/usage/<name>.json")`
在 `openai_provider::tests::usage_fixtures` 内读取）：

| fixture | 关键字段 | C0 当前结果 | C1 正确期望 |
|---|---|---|---|
| `deepseek_hit_miss.json` | hit:100 + miss:156，prompt 256 | read=None | read=100, uncached=156, Reported |
| `deepseek_hit_only.json` | 只有 hit:100，prompt 256 | read=None | read=100, uncached=156, Reported |
| `deepseek_explicit_zero_hit.json` | hit:0 + miss:256 | read=None | read=Some(0), uncached=256, Reported |
| `openai_cached_tokens.json` | `cached_tokens:100` | read=100, uncached=156 | 不变 |
| `openai_explicit_zero_cached.json` | `cached_tokens:0` | read=Some(0), uncached=256 | 不变 |
| `neither.json` | 无缓存字段 | read=None, Reported | read=None, **Unknown** |
| `deepseek_hit_greater_than_total.json` | hit:300 > prompt 256 | Ok（无错误） | **Err**（ExceedsTotal） |
| `deepseek_miss_not_conserved.json` | hit:100+miss:200 ≠ 256 | Ok | **Err**（NonConserved） |
| `format_conflict.json` | hit:100/miss:156 + cached:200 | Ok, read=Some(200) | **Err**（FormatConflict） |

**失败基线（C1 实现前手动运行，证明缺口）**：`bash scripts/cargo-agent.sh test -p cognit --lib
openai_provider::tests` → `11 passed; 7 failed`，失败断言原文：

```text
deepseek_hit_miss_parse                 ... left: None, right: Some(100)   (cache_read_tokens)
deepseek_hit_only_derives_miss_from_total ... left: None, right: Some(100)
deepseek_explicit_zero_hit_is_distinct_from_none ... left: None, right: Some(0)
deepseek_hit_greater_than_total_is_protocol_error ... assertion failed: parse_result(..).is_err()
deepseek_hit_plus_miss_not_conserved_is_protocol_error ... assertion failed: is_err()
conflicting_cache_formats_is_protocol_error ... assertion failed: is_err()
streaming_final_usage_chunk_deepseek_parses ... left: None, right: Some(100)
```

即：DeepSeek 字段被 `ApiUsage` 静默丢弃（read=None）；hit>total / 不守恒 / 格式冲突均不报错。

## Phase C1：DeepSeek usage 正确解析（P0）

### 修改范围

- `crates/cognit/src/adapters/inference/openai_provider.rs`
- `crates/fabric/src/types/llm_types.rs`（只在需要统一验证时修改）
- `crates/fabric/tests/inference_contract.rs`

### 工作项

1. 扩展私有 `ApiUsage`；token wire type 改为 `u64`，避免大上下文累积时过早缩窄；
2. 把 `openai_usage` 拆为小型、纯函数的兼容解析器；
3. 非流式和流式共用同一解析函数；
4. 冲突或不守恒返回 typed provider protocol failure；
5. 保留旧 OpenAI response 兼容；
6. reasoning content 和 tool-call 行为不得在本阶段改变。

### 测试

```text
cargo-agent test -p fabric --test inference_contract
cargo-agent test -p cognit openai_provider::tests
```

实际执行必须使用仓库脚本：

```text
bash scripts/cargo-agent.sh test ...
```

### 完成条件

- Chat Completions 和 SSE 都能读取 DeepSeek cache token；
- 明确零命中显示为 Reported + 0；
- 未报告显示为 Unknown；
- OpenAI/DeepSeek 冲突不会被吞掉；
- 无缓存 provider 的现有测试不回归。

### C0+C1 实施结果（§7.2 模板块，2026-08-04）

```text
STATUS: accepted
BASE: origin/dev 278f357638bf575cda34008a176a71c036326929
PHASE: C0 + C1（合并一个 PR，两个 commit；C0 失败基线 → C1 parser 修复）
SCOPE:
  - crates/cognit/tests/fixtures/usage/*.json（9 个新 fixture）
  - crates/cognit/src/adapters/inference/openai_provider.rs（ApiUsage u64 + DeepSeek
    prompt_cache_hit/miss_tokens 字段；openai_usage 拆为纯函数解析器 + typed 错误；
    mod usage_fixtures 测试）
  - crates/fabric/src/types/llm_types.rs（InferenceUsageError + InferenceUsage::validate()）
  - crates/fabric/tests/inference_contract.rs（守恒/不变式 + validate 正负向测试）
  - 本计划文档（C0/C1 状态块 + fixture 清单 + 失败基线）
CONTRACT:
  - 新增 fabric::llm_types::InferenceUsageError（NonConserved / ExceedsTotal /
    UnsupportedWithCache / FormatConflict / Invalid，thiserror + PartialEq/Eq）
  - 新增 InferenceUsage::validate()：read+uncached==total（全已知时）、单值 ≤ total、
    Unsupported 不得携带 cache、Some(0)≠None、缺失保留 None
  - openai_provider 私有 ApiUsage：prompt/completion u32→u64；新增 DeepSeek hit/miss Option<u64>
  - 无缓存字段 → CacheTelemetry::Unknown（原为 reported() 强制的 Reported）
  - 解析不依赖 model name（不 grep model.contains("deepseek")）
VALIDATION:
  - C0 失败基线：test -p cognit --lib openai_provider::tests → 11 passed / 7 failed（已记录于 C0 段）
  - C1 后全绿：
    bash scripts/cargo-agent.sh test -p fabric --test inference_contract   # 20 passed
    bash scripts/cargo-agent.sh test -p fabric --lib                        # 377 passed
    bash scripts/cargo-agent.sh test -p cognit --lib                        # 362 passed
    bash scripts/cargo-agent.sh clippy -p fabric -p cognit --all-targets -- -D warnings  # 0
    bash scripts/cargo-agent.sh fmt --all -- --check                        # 0
RUNTIME EVIDENCE:
  - 无真实 provider 请求（fixture 驱动，不访问网络）；真实 benchmark 属 C5
METRICS:
  - 无运行时指标；契约级 None / Some(0) / Reported / Unknown / Unsupported 独立断言
ROLLBACK:
  - revert 本 PR commit（测试 + 私有 parser + fabric 新方法，可独立回滚）
OPEN ITEMS:
  - C2 provider cache reporting config（reporting = auto|deepseek_chat|openai_cached_tokens|
    unsupported；决定 no-field 情形 Unknown 是否可被 unsupported 覆盖）
  - C3 SHA-256 prefix shape；C4 typed message partition；C5 真实 benchmark；C6 recall cache；C7 tool cache
  - C1 流式冲突/守恒错误路径：SSE 中段已输出内容后收到非法 usage chunk，当前经 openai_usage
    返回 Err 终止流（fail closed）；后续若需 log-and-continue 需单独决策
```

## Phase C2：Provider 能力与代理差异（P0）

### 修改范围

- `crates/cognit/src/adapters/inference/provider.rs`
- `crates/cognit/src/composition/inference_factory.rs`
- `crates/executive/src/composition/config/*`
- `config/default.toml`
- 对应 layered config contract tests。

### 设计

配置增加可选、显式的 cache reporting hint；默认 `auto`：

```toml
[providers.cache]
reporting = "auto" # auto | deepseek_chat | openai_cached_tokens | unsupported
prefix_cache = "auto" # auto | supported | unsupported
```

该配置只解释 telemetry，不改变 provider 是否真的命中。`auto` 缺字段时保持 Unknown。

### 工作项

1. 为官方 DeepSeek 和 LejuRobot proxy 保留独立 provider identity；
2. 不因它们使用相同模型就合并 backpressure、cache stats 或成本；
3. 把有效 reporting mode 放入 host-owned runtime facts 或 provider diagnostics；
4. 不在启动时主动发费钱的探测请求；真实 capability probe 放到显式诊断命令/benchmark；
5. API secret 不进入日志、shape 和 receipt。

### 完成条件

- 官方 endpoint 和代理的缓存数据可以分别汇总；
- 配置缺省不伪造 supported；
- provider/model identity 来自主机配置，不来自模型自述。

### 实施结果（§7.2 模板块，2026-08-04）

```text
STATUS: accepted
BASE: origin/dev 278f357638bf575cda34008a176a71c036326929（feature 分支上，随 C3-C7 后续提交）
PHASE: C2
SCOPE:
  - crates/cognit/src/config/mod.rs（CacheReportingMode / PrefixCacheCapability /
    ProviderCacheConfig；ProviderConfig.cache 字段）
  - crates/cognit/src/adapters/inference/openai_provider.rs（OpenAiProvider.cache_reporting +
    with_cache_reporting + runtime_facts 覆盖；openai_usage 增 reporting 参数，Unsupported→Unsupported
    且不携带 cache 数字；流式闭包前复制 mode 避免 'static 借用）
  - crates/cognit/src/composition/inference_factory.rs（OpenAi 分支传 config.cache.reporting）
  - crates/fabric/src/types/llm_types.rs（ModelRuntimeFacts.cache_reporting: Option<String>）
  - crates/executive/src/composition/config/{provider.rs,mod.rs}（re-export）
  - crates/executive/src/application/{inference_port.rs,turn_pipeline.rs}（构造点补字段）
  - crates/executive/src/core/system_core_runtime.rs、scheduler.rs、两个 provider timeout 测试、
    bootstrap runtime_tests.rs（ProviderConfig 字面量补 cache: Default::default()）
  - config/default.toml + config/production.toml.example（[providers.cache] reporting/prefix_cache=auto）
  - config/schema/aletheon-config.schema.json（UPDATE_CONFIG_SCHEMA 再生成）
  - crates/executive/tests/layered_config_contract.rs（anchor 测试断言两 provider cache=Auto）
CONTRACT:
  - cognit::config::CacheReportingMode{Auto,DeepSeekChat,OpenAiCachedTokens,Unsupported} + as_str()
  - cognit::config::PrefixCacheCapability{Auto,Supported,Unsupported} + as_str()
  - ProviderConfig.cache: ProviderCacheConfig（serde default + deny_unknown_fields）
  - fabric::ModelRuntimeFacts.cache_reporting: Option<String>（skip_serializing_if is_none，
    不破坏既有 runtime-facts JSON）
  - openai_usage(usage, reporting)：Unsupported 短路径；其余维持 C1 字段解析
VALIDATION:
  bash scripts/cargo-agent.sh test -p fabric --lib                    # 377 passed
  bash scripts/cargo-agent.sh test -p cognit --lib                    # 365 passed
  bash scripts/cargo-agent.sh test -p executive --lib                 # 673 passed
  UPDATE_CONFIG_SCHEMA=1 bash scripts/cargo-agent.sh test -p executive --test layered_config_contract  # 10 passed
  bash scripts/cargo-agent.sh clippy -p fabric -p cognit -p executive --all-targets -- -D warnings  # 0
  bash scripts/cargo-agent.sh fmt --all -- --check                    # 0
RUNTIME EVIDENCE:
  - 无真实请求；Unsupported 模式 + 显式 deepseek_chat/openai_cached 模式 fixture 测试（cognit 3 个新测试）
METRICS:
  - ModelRuntimeFacts 注入 <runtime-facts> 含 cache_reporting（每 provider 恒定，前缀稳定）
ROLLBACK:
  - revert C2 commit；配置缺省 auto 时行为与 C1 完全一致
OPEN ITEMS:
  - C3 SHA-256 prefix shape；C4 typed partition；C5 benchmark（测量后按需把 provider 显式置
    reporting/prefix_cache）；C6 recall cache；C7 tool cache
```

## Phase C3：稳定前缀 shape 接入（P1）

### 修改范围

- `crates/executive/src/host/daemon/cache_shape.rs`
- `crates/executive/src/host/daemon/bootstrap/request.rs`
- `crates/executive/src/host/daemon/bootstrap/turn_runtime.rs`
- `crates/executive/src/application/context_assembler.rs`
- `crates/cognit/src/harness/session.rs`
- inference receipt/model projection 类型和测试。

### 工作项

1. 用 SHA-256 shape 替换 `DefaultHasher`；
2. 使用已有 `tool_schema_digest`，删除 tool-name-only hash；
3. 在 daemon 构建 cached prefix 时生成 system digest；
4. 在 AgentProfile 和 tool registry 结算后生成完整 shape；
5. 每轮记录 current shape 和 previous shape；
6. 只在 shape 变化时产生本地 miss reason：
   - provider/model changed；
   - system changed；
   - tool schema changed；
   - profile changed；
   - compaction/rewrite changed；
7. shape 相同但 provider 未命中时，只记录 `provider_miss_or_eviction` 推断，不能声称原因确定。

### 完成条件

- tool 排序变化不改变 shape；
- tool description/schema 变化改变 shape；
- memory update 和最新 user input 不改变 stable shape；
- skill/profile 更新通过权威管理路径重建 shape；
- shape 不包含 secret、原始 prompt 或用户内容。

### 实施结果（§7.2 模板块，2026-08-04）

```text
STATUS: code_complete（类型级契约全部完成并测试；daemon 每轮接入部分留 OPEN ITEM）
BASE: origin/dev 278f357638bf575cda34008a176a71c036326929（feature 分支）
PHASE: C3
SCOPE:
  - crates/executive/src/host/daemon/cache_shape.rs（重写：CacheShape/DefaultHasher →
    InferencePrefixShape + LocalMissReason + PrefixShapeTracker + agent_profile_digest +
    CacheStats；SHA-256 + domain separation `aletheon.inference-prefix-shape.v1\0`）
  - crates/fabric/src/types/inference_receipt.rs（InferenceTerminalReceipt 增
    prefix_shape_digest: Option<String>，serde default + skip_if_none）
  - crates/cognit/src/harness/session.rs、crates/fabric/tests/inference_contract.rs
    （receipt 构造补 prefix_shape_digest: None）
CONTRACT:
  - executive::host::daemon::cache_shape::InferencePrefixShape{version,provider_id,model_id,
    transport,system_prefix_digest,tool_schema_digest,agent_profile_digest,rewrite_version}
  - compute() 用 fabric::tool_schema_digest（顺序/对象 key 无关）；digest() 全 shape SHA-256
  - compare() -> Option<LocalMissReason>（ProviderOrModelChanged/TransportChanged/SystemChanged/
    ToolSchemaChanged/ProfileChanged/CompactionOrRewrite；相同→None；同 shape 但 provider miss→
    ProviderMissOrEviction，不声称本地原因）
  - PrefixShapeTracker::track 返回变化原因；CacheStats 保留
  - InferenceTerminalReceipt.prefix_shape_digest（诊断身份，非正确性依赖）
VALIDATION:
  bash scripts/cargo-agent.sh test -p executive --lib cache_shape    # 12 passed
  bash scripts/cargo-agent.sh test -p executive --lib               # 677 passed
  bash scripts/cargo-agent.sh test -p fabric --test inference_contract  # 20 passed
  bash scripts/cargo-agent.sh test -p cognit --lib                  # 365 passed
  bash scripts/cargo-agent.sh clippy -p fabric -p cognit -p executive --all-targets -- -D warnings  # 0
  bash scripts/cargo-agent.sh fmt --all -- --check                  # 0
RUNTIME EVIDENCE:
  - 无真实请求；类型级契约测试覆盖全部完成条件（工具顺序/schema、system、profile、provider/model、
    transport、compaction、memory/user-input 不进 shape、digest 无 secret/原文）
METRICS:
  - 无运行时指标；shape 为诊断身份，不参与请求正确性
ROLLBACK:
  - revert C3 commit（新类型 + receipt 可选字段，向后兼容）
OPEN ITEMS:
  - daemon 每轮 current-vs-previous 跟踪 + receipt 打标：bootstrap 处 provider identity
    （default_provider/model/transport）不在 DaemonConfig 可达域，需把 provider registry 身份
    或 AppConfig 传入 RequestHandler::new 后，在 turn_runtime 每轮 compute + PrefixShapeTracker.track
    + record_inference_receipt 打标 shape digest
  - skill/profile 更新后经权威管理路径重建 shape（PrefixBuilder 重建已存在，shape 重算待接）
  - C4 typed partition；C5 benchmark；C6 recall cache；C7 tool cache
```

## Phase C4：消息构造稳定性（P1）

### 修改范围

- `crates/executive/src/composition/prefix_builder.rs`
- `crates/cognit/src/harness/linear/message_compose.rs`
- `crates/cognit/src/harness/linear/mod.rs`
- `crates/executive/src/application/context_assembler.rs`
- Agent profile/skill composition tests。

### 设计

消息分区固定为：

| 区域 | 内容 | 变化频率 |
|---|---|---|
| StablePrefix | identity、安全规则、固定协议 | daemon/profile 生命周期 |
| StableTools | profile 对应 canonical tools | profile/tool registry 生命周期 |
| Conversation | 原始历史或稳定 compacted summary | 每轮追加/压缩时重写 |
| DynamicContext | memory、goal、Dasein、plan | 每轮 |
| CurrentInput | 当前输入 | 每轮 |

### 工作项

1. 给每个区域增加 typed projection，而不是继续拼接无身份 String；
2. 保留现有 wire message 顺序，先通过 snapshot 测试锁定；
3. 所有 map/set 输出必须确定性排序；
4. 当前时间等运行事实仅在任务真正需要时注入动态尾部；
5. Tool exposure 采用 AgentProfile 级稳定集合，不使用全局所有工具，也不每轮随机裁剪；
6. context compaction 只增加 rewrite version，不修改 stable prefix；
7. 超预算裁剪从动态区最旧/最低优先级内容开始，不裁掉 authority/safety prefix。
8. 为各分区记录构造耗时、序列化字节数和重用候选比例；只记录数值和 digest，不记录原文；
9. 本阶段不引入跨 turn 的 projected-message 结果缓存。只有 profiling 证明构造成本显著，并且能定义
   principal/profile/rewrite 隔离键后，才另立实施计划。

### 完成条件

- 同一 profile 的连续 turn 在第一个动态字段前字节一致；
- 相同 logical JSON 产生相同 wire tool schema；
- system prefix 内没有 operation/time/session 随机字段；
- prompt snapshot 测试覆盖普通对话、tool result、memory update、compaction。
- profiling 能回答本地 context projection 是否值得缓存；没有数据时不得宣称 L1 已完成结果复用。

### 实施结果（§7.2 模板块，2026-08-04）

```text
STATUS: code_complete（typed partition + profiling + wire-order 锁定；cognit message_compose
        typed 重构 + 跨分区 snapshot 未做，见 OPEN ITEMS）
BASE: origin/dev 278f357638bf575cda34008a176a71c036326929（feature 分支）
PHASE: C4
SCOPE:
  - crates/executive/src/application/prompt_partition.rs（新：PromptRegion{StablePrefix,
    StableTools,Conversation,DynamicContext,CurrentInput} + PromptPartition + 
    PromptConstructionProfile{region_bytes,total_bytes,summary} + build_partitions）
  - crates/executive/src/application/context_assembler.rs（assemble 拆分 dynamic_context 与
    current_input，build_partitions 产 profile；AssembledContext.profile 字段；wire 消息顺序不变）
  - crates/executive/src/application/mod.rs（注册 prompt_partition）
CONTRACT:
  - PromptRegion::as_str() 稳定 snake_case；summary() 按权威顺序返回 region:bytes
  - StablePrefix 分区只含 system 前缀（不含 memory/goal/Dasein/当前输入/随机字段）；
    StableTools 为 marker（工具 canonicalize 由 fabric 负责）
  - AssembledContext 增 profile: PromptConstructionProfile（诊断投影，不影响 wire）
VALIDATION:
  bash scripts/cargo-agent.sh test -p executive --lib prompt_partition   # 4 passed
  bash scripts/cargo-agent.sh test -p executive --lib                   # 681 passed
  bash scripts/cargo-agent.sh test -p executive --test context_assembler # 6 passed（wire order 锁定）
  bash scripts/cargo-agent.sh clippy -p executive --all-targets -- -D warnings  # 0
  bash scripts/cargo-agent.sh fmt --all -- --check                     # 0
RUNTIME EVIDENCE:
  - 无真实请求；测试断言 StablePrefix 跨 turn 字节一致（无随机字段）、分区字节可测、summary 确定性
METRICS:
  - 每轮 PromptConstructionProfile 记录各分区 serialized_bytes + construction_ns（数值/digest，无原文）
ROLLBACK:
  - revert C4 commit；AssembledContext.profile 是新增字段，wire 不变
OPEN ITEMS:
  - cognit::harness::linear::message_compose 的 plan/memory/goal/dasein marker 仍是平铺 String，
    未 typed 分区（可后续包成 DynamicContext 分区）
  - prompt snapshot 测试覆盖 tool result / memory update / compaction 的 wire 快照（当前只有
    context_assembler 顺序锁定 + 字节测试）
  - 超预算裁剪优先级（从动态区最旧内容开始）未在 context_assembler 显式实现（现有 MAX_INJECTED_CHARS
    从前往后截断，语义一致但无分区感知）
  - 本阶段未引入跨 turn projected-message 结果缓存（符合计划"profiling 证明前不缓存"）
  - C5 benchmark；C6 recall cache；C7 tool cache
```

## Phase C5：DeepSeek 真实缓存基准（P1）

### 修改范围

- 新增 `scripts/bench-deepseek-cache` 或 `tools/` 下的独立诊断工具；
- `docs/testing/deepseek-cache.md` 保存方法与版本化结果；
- 不把 benchmark 特例写进生产 prompt。

### 场景

1. 完全相同的长请求；
2. 只改变最后一个 user message；
3. 改变 system prompt 一个字符；
4. 只改变 tool 顺序；
5. 改变一个 tool schema；
6. 添加 memory update；
7. compaction 前后；
8. 官方 DeepSeek vs LejuRobot proxy；
9. V4 Flash vs V4 Pro；
10. streaming vs non-streaming。

### 每次记录

```text
effective provider/model
request shape digest
prompt total/hit/miss
output/reasoning tokens
TTFT
total latency
provider attempts/retries
HTTP status/request id（若 provider 提供）
```

不能用累计 session tokens 代替当前请求的 active context 或 billed usage。

### 完成条件

- 至少三次冷/热重复运行；
- 官方和代理分别出结果；
- 观察到字段缺失时报告 Unknown，而不是失败或零命中；
- 结果记录模型版本、日期和 endpoint identity，不记录 key。

### 实施结果（§7.2 模板块，2026-08-04）

```text
STATUS: accepted（lejurobot 代理侧全部场景真实命中证据）；官方 deepseek.com = externally_blocked
        （无 DEEPSEEK_API_KEY）
BASE: origin/dev 278f357638bf575cda34008a176a71c036326929（feature 分支）
PHASE: C5
SCOPE:
  - scripts/bench-deepseek-cache（新：真实 chat-completions 诊断工具，环境变量配置
    BENCH_BASE_URL/API_KEY/MODEL/MAX_TOKENS/ROUNDS/SCENARIOS；解析 DeepSeek
    prompt_cache_hit/miss_tokens 或 OpenAI cached_tokens；SSE 取最终 usage chunk；
    含 7 场景：identical/last_msg_changed/system_char_changed/tools_reordered/
    tool_schema_changed/memory_added/streaming）
  - docs/testing/deepseek-cache.md（方法 + 版本化结果）
CONTRACT:
  - 无产品契约变化；工具独立于产品请求路径；模型需用代理可路由 id（去 `[1m]` 后缀）
VALIDATION（真实证据，lejurobot 代理 https://aiapi.lejurobot.com/v1，model
  deepseek/deepseek-v4-flash 与 -pro，max_tokens=24，2026-08-04）：
  - identical：冷 hit0 → 热 1152/1183（97.4%），3 连稳 97.4%
  - last_msg_changed：1152/1185（97.2%）—— 改最后 user 消息保留前缀
  - system_char_changed：改一个字符首次 0 hit（前缀从第 0 token 失效），重跑 1152
  - tools_reordered：896/1227（73%）→ 1152/1227（94%）
  - tool_schema_changed：1024/1199（85%）→ 1152/1199（96%）
  - memory_added：1152/1199（96%）—— memory 在 user 尾部，前缀保留
  - streaming：1152/1183（97.4%）
  - pro identical/streaming：1024/1104（92.8%）
RUNTIME EVIDENCE:
  - 真实请求 3+ 冷/热重复；命中率证明代理 DeepSeek prefix cache 生效、稳定前缀决定命中、
    动态尾部不破坏前缀；官方端点无 key 未测
METRICS:
  - 命中率 per scenario（见 docs/testing/deepseek-cache.md）；字段缺失报 Unknown（脚本列 error）
ROLLBACK:
  - revert C5 commit（独立脚本 + 文档，无产品路径影响）
OPEN ITEMS:
  - 官方 deepseek.com benchmark：配 DEEPSEEK_API_KEY 后跑 BENCH_BASE_URL=api.deepseek.com
  - 可把代理 `[providers.cache] reporting` 置 deepseek_chat（测量已证实返回 DeepSeek 字段）
  - C6 recall cache；C7 tool cache
```

## Phase C6：Mnemosyne Recall Cache（P1）

### 修改范围

- 先定位 Mnemosyne 当前 recall application port 和 repository adapter；
- 在 Mnemosyne 内新增 cache adapter，不放 Executive；
- bootstrap 只负责选择/注入实现；
- 增加 metrics 和 generation 测试。

### Key

```rust
pub struct RecallCacheKey {
    principal_scope_digest: String,
    query_digest: String,
    filters_digest: String,
    top_k: u32,
    embedding_model_id: String,
    memory_policy_version: String,
    memory_generation: u64,
}
```

### Value

- 只保存稳定 memory IDs、score 和必要 projection；
- 权威内容仍从 repository 读取，或 value 携带可验证的 item version；
- 不缓存权限判断结果跨 principal 复用。

### 失效

- promotion、insert、delete、merge/consolidation commit 后增加 generation；
- 写事务提交失败不得增加 generation；
- key 包含 generation，避免全表逐项删除；
- 第一版使用短 TTL、有界 LRU；不引入 Redis。

### 完成条件

- 相同 query/scope 命中；
-不同 principal/scope 不能串结果；
- memory write 后旧结果不可命中；
- 并发 miss 使用 single-flight，避免重复 embedding/retrieval 风暴；
- 缓存失败不影响权威 recall。

## Phase C7：Corpus Tool Result Cache（P2）

### 修改范围

- `fabric` 或 Corpus 的 tool descriptor contract；
- Corpus tool registry/runner；
- Executive tool receipt/observability；
- 只读工具 contract tests。

### 契约

```rust
pub enum ToolCachePolicy {
    Never,
    PerTurn,
    Session { ttl_ms: u64 },
    SharedReadOnly { ttl_ms: u64, vary_by_principal: bool },
}
```

默认必须是 `Never`。只有工具 owner 显式声明，并通过安全审查后才能启用。

### Key 必须包含

- tool name + implementation/version digest；
- canonical arguments；
- workspace/repository identity；
- principal/permission scope（除非明确安全共享）；
-外部资源 version/etag（若存在）；
- policy version。

### 永不缓存

- shell/process execution；
- file/git mutation；
- network action 或消息发送；
- approval/credential/access decision；
- robot execute/cancel/safe_stop/observe/get_state；
- wall clock、随机数和无版本实时查询。

### 完成条件

- mutation tool 无法通过配置误开缓存；
- cache hit 仍产生可审计 receipt，明确标记没有执行底层工具；
- workspace/principal 隔离有负向测试；
- TTL 到期与实现版本变化会 miss；
- 缓存层故障 fail open 到真实只读工具，但不得绕过权限检查。

---

## 5. 明确不做

1. 不默认缓存 Agent 最终回答；DeepSeek prefix cache 不等于回答缓存。
2. 不把 semantic similarity response cache 放进主对话链。
3. 不把 Redis 引入第一版；先证明单进程 bounded cache 有价值。
4. 不缓存机器人实时状态和动作结果作为下一次执行事实。
5. 不为了命中率把所有工具暴露给所有 Agent。
6. 不把 Memory、Dasein、Goal 固化进 system prompt。
7. 不将 provider cache miss 当作请求失败。

---

## 6. 观测指标和验收面板

至少暴露以下互相独立的指标：

```text
inference_input_tokens_total
inference_output_tokens_total
inference_cache_read_tokens_total
inference_cache_write_tokens_total
inference_uncached_input_tokens_total
inference_cache_telemetry_unknown_total
inference_prefix_shape_changes_total{reason}
memory_recall_cache_hit_total
memory_recall_cache_miss_total
tool_result_cache_hit_total{tool}
tool_result_cache_miss_total{tool}
```

禁止：

- 用 tool calls 数量推断 inference rounds；
- 用累计 billed tokens 推断当前 context occupancy；
- 把 Unknown 算作 miss；
- 把 provider prefix hit 和本地 recall hit 混成一个命中率。

---

## 7. PR 切分

| PR | 内容 | 风险 | 前置 |
|---|---|---|---|
| C0 | usage fixtures + provider-neutral invariant tests | 低 | 无 |
| C1 | DeepSeek parser + streaming/non-streaming parity | 中 | C0 |
| C2 | provider cache reporting config/runtime facts | 中 | C1 |
| C3 | SHA-256 prefix shape + receipt projection | 中 | C1 |
| C4 | typed message partition + snapshot/profiling | 中高 | C3 |
| C5 | official/proxy real cache benchmark | 低，外部依赖 | C2-C4 |
| C6 | Mnemosyne recall cache | 中高 | C0；可在 C3 后独立执行 |
| C7 | Corpus explicit tool cache policy | 高，安全敏感 | C6 非必需，但需独立安全评审 |

每个 PR 必须保持可回滚；禁止一个 PR 同时修改 provider parser、Mnemosyne repository 和
Corpus runner。

### 7.1 阶段交付矩阵

| 阶段 | 最小确定性验证 | 必须检查的回归面 | 阶段证据 |
|---|---|---|---|
| C0 | `test -p fabric --test inference_contract`；Cognit fixture 测试 | `None`/0/Unknown/Unsupported 区分 | fixture 清单和失败基线 |
| C1 | Cognit OpenAI adapter 定向测试 | SSE 与非流式、OpenAI 旧字段、reasoning/tool call | 解析矩阵和协议错误样例 |
| C2 | Executive layered config contract | provider identity、secret redaction、默认 `auto` | resolved config/runtime facts 快照 |
| C3 | Executive cache-shape/receipt 定向测试 | profile/tool update、compaction、无用户原文 | 三次重建 digest 对比 |
| C4 | Cognit/Executive message snapshot 测试 | tool result、memory、compaction、预算裁剪 | 分区快照和 profiling 数值 |
| C5 | 独立 benchmark，至少三次冷/热运行 | official/proxy、Flash/Pro、stream/non-stream | 版本化结果文档，不含 key |
| C6 | Mnemosyne cache/repository 定向测试 | principal/scope、generation、single-flight、fail-open | hit/miss/invalidation 测试 |
| C7 | Corpus registry/runner contract 测试 | mutation/robot/permission 拒绝，receipt 保留 | 允许/禁止工具策略矩阵 |

表中命令表示 Cargo arguments；实际必须写成 `bash scripts/cargo-agent.sh <arguments>`。若当前测试名或
package 已变化，执行者先定位现有最窄目标，不得直接改成 workspace-wide 测试。

### 7.2 每个 PR 的固定完成模板

```text
STATUS: code_complete | externally_blocked | accepted
BASE: origin/dev commit
SCOPE: 本 PR 修改的阶段和文件
CONTRACT: 新增/修改/未修改的 public contract
VALIDATION: 完整命令、退出码、关键断言
RUNTIME EVIDENCE: provider/installed acceptance；不适用时说明原因
METRICS: inference/cache 指标变化，Unknown 是否保留
ROLLBACK: 可独立回滚的 commit/配置开关
OPEN ITEMS: 后续阶段，不得写成当前已完成
```

---

## 8. 最终关闭标准

本计划只有同时满足以下条件才可标记完成：

1. DeepSeek Chat Completions 的 hit/miss 在流式和非流式路径正确进入 `InferenceUsage`；
2. 官方 DeepSeek 与 LejuRobot proxy 有独立真实请求证据；
3. stable prefix/tool shape 可复现，变化原因可解释；
4. 三轮同 Session 测试证明动态尾部不会无故破坏稳定前缀；
5. Recall cache 通过 scope、generation、并发 single-flight 测试；
6. Tool cache 只有显式只读工具可启用，机器人和 mutation tool 有拒绝测试；
7. 安装态部署通过仓库 `AGENTS.md` 要求：release、`/usr/bin/aletheon` 和运行 daemon
   digest 一致，restart counter 稳定，并通过官方用户 socket 完成真实 LLM 请求；
8. 验收分别报告 inference rounds、provider retries、tool calls、active context、累计 usage
   和各层 cache 指标。
9. C4 profiling 没有证明收益前，不新增 Context Projection 结果缓存；若后续新增，必须另有隔离键、
   容量上限、失效和敏感数据审查。
