# DeepSeek 缓存与消息优化合同

> 状态：C0-C7 已完成并合入 PR #182。
> 生产路由：LejuRobot OpenAI-compatible proxy；直接访问 DeepSeek 官方端点仅作可选诊断。
> 基准工具：`scripts/bench-deepseek-cache`（真实请求诊断，非产品请求路径）。

本文保留缓存优化的长期行为合同、实现锚点和可复现基准。临时任务排序与执行状态留在 Git 历史中。

## Provider telemetry normalization

Provider cache usage 只能来自 wire usage 字段和 effective `CacheReportingMode`，不能由模型名称推断。
`openai_usage`（`crates/cognit/src/adapters/inference/openai_provider.rs`）按以下顺序归一化：

1. `Unsupported` 不记录缓存数字；
2. DeepSeek hit/miss 同时存在时校验守恒后使用；
3. 只有 DeepSeek hit 且总量已知时计算 miss；
4. OpenAI `cached_tokens` 映射为 read，剩余为 uncached；
5. 两种格式同时存在时必须一致，否则返回 typed protocol error；
6. provider 未返回可解释字段时保持 `Unknown`，不伪造零命中。

累计 provider usage、活动上下文占用、cache usage、inference rounds、retries 与 tool calls 是独立指标，不能互相推导。

## Prompt partition and stable prefix

`PromptRegion`（`crates/executive/src/application/prompt_partition.rs`）把请求分成稳定身份/协议、稳定工具、会话历史、动态上下文和当前输入。稳定区不能包含时间、UUID、operation ID、预算、设备状态、per-turn recall 或 attempt number。

分区与 `InferencePrefixShape` 只提供诊断身份和构造成本观测，不改变发送给 provider 的内容，也不作为本地命中判定。生产请求保持稳定前缀字节序；动态 memory、goal、plan 和当前输入位于其后。

## Generation-keyed recall cache

`mnemosyne::recall_cache` 使用 principal scope、query、filters、embedding model、memory-policy version 和 write generation 组成 key。任何成功写入都会推进 generation，因此旧结果不再匹配；并发 miss 使用 single-flight。缓存故障回退到权威 `MemoryService`，缓存本身不作权限或 authority 决策。

## Read-only tool result cache

`corpus::tools::read_only_cache` 只服务同时满足以下条件的工具：

- tool 显式声明非 `Never` 的 `ToolCachePolicy`；
- executor 已确认 `PermissionLevel::L0`；
- key 绑定 tool/version、canonical arguments、workspace 以及 policy 要求的 principal/session/turn scope。

命中仍产生带 `served_from_cache` 的可审计 `CapabilityResult`，且权限/approval gate 先于缓存。缓存失败回退真实只读调用；mutating tool 不能通过配置进入缓存。

## Live provider benchmark

2026-08-04 在 LejuRobot 代理 `https://aiapi.lejurobot.com/v1` 上运行：

```bash
BENCH_MAX_TOKENS=24 BENCH_ROUNDS=2 \
  BENCH_MODEL=deepseek/deepseek-v4-flash \
  bash scripts/bench-deepseek-cache
```

| 场景 | run1 | run2 | 结论 |
|---|---|---|---|
| identical（flash） | 1183 total / 0 hit / 1183 miss | 1183 / 1152 / 31 | 冷到热 97.4% |
| last message changed | 1185 / 1152 / 33 | 1185 / 1152 / 33 | 动态尾部保留稳定前缀 |
| system char changed | 1184 / 0 / 1184 | 1184 / 1152 / 32 | 稳定前缀变化使首次命中失效 |
| tools reordered | 1227 / 896 / 331 | 1227 / 1152 / 75 | 工具顺序变化破坏部分前缀 |
| tool schema changed | 1199 / 1024 / 175 | 1199 / 1152 / 47 | schema 变化破坏部分前缀 |
| memory added | 1199 / 1152 / 47 | 1199 / 1152 / 47 | 动态 memory 不破坏前缀 |
| streaming（flash） | 1183 / 1152 / 31 | 1183 / 1152 / 31 | SSE 最终 usage 一致 |
| identical（pro） | 1104 / 0 / 1104 | 1104 / 1024 / 80 | Pro 热命中 92.8% |
| streaming（pro） | 1104 / 1024 / 80 | 1104 / 1024 / 80 | Pro streaming 一致 |

这些数字证明该代理当时的 prefix cache 行为，不是永久 SLA。本地 `InferencePrefixShape` 不决定 provider 命中；identical 热请求仍有 31 miss token，属于 provider 侧 miss/eviction 行为。

默认 `[providers.cache] reporting = "auto"` 可解析代理返回的 DeepSeek 字段；需要固定协议时可显式设置 `reporting = "deepseek_chat"`。

## Optional direct DeepSeek diagnostic

生产验收使用配置中的 LejuRobot 路由。没有 `DEEPSEEK_API_KEY` 不阻塞生产验收；若要比较官方端点，可单独运行：

```bash
BENCH_BASE_URL=https://api.deepseek.com/v1 \
BENCH_API_KEY="$DEEPSEEK_API_KEY" \
BENCH_MODEL=deepseek-chat \
bash scripts/bench-deepseek-cache
```

该命令消耗真实 token，结果只能作为 provider 诊断，不能替代 installed runtime、official socket 或真实应用请求的验收。
