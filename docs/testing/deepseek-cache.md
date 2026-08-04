# DeepSeek 提示词缓存基准（C5）

> 工具：`scripts/bench-deepseek-cache`（真实请求诊断，非产品请求路径）。
> 方法：对每个场景发 `ROUNDS` 次 chat-completions 请求，读取 provider 返回的
> `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`（DeepSeek 字段）或
> `prompt_tokens_details.cached_tokens`（OpenAI 字段）。命中率 = hit / total。
> 这是 LejuRobot 代理侧 prefix-cache 的真实测量；`model` 需用代理可路由的 id
> （`deepseek/deepseek-v4-flash`，不带 `[1m]` 窗口后缀）。

## 结果（2026-08-04，lejurobot 代理 `https://aiapi.lejurobot.com/v1`）

`bash scripts/bench-deepseek-cache`，`BENCH_MAX_TOKENS=24`，`BENCH_ROUNDS=2`。
列：`场景  run1(total hit miss output)  run2(...)`。

| 场景 | run1 | run2 | 结论 |
|---|---|---|---|
| identical（flash） | 1183 · 0 · 1183 | **1183 · 1152 · 31** | 冷→热 **97.4% 命中**；代理 prefix cache 生效 |
| last_msg_changed | **1185 · 1152 · 33** | **1185 · 1152 · 33** | 改最后一条 user 消息**保留**缓存前缀（97.2%） |
| system_char_changed | 1184 · 0 · 1184（首次） | 1184 · 1152 · 32 | 改 system 一个字符 → 前缀从第 0 token 失效，重跑恢复 |
| tools_reordered | 1227 · 896 · 331 | **1227 · 1152 · 75** | 工具顺序变化 → 部分命中（73%），二次热（94%） |
| tool_schema_changed | 1199 · 1024 · 175 | **1199 · 1152 · 47** | schema 变化 → 部分命中（85%），二次热（96%） |
| memory_added | **1199 · 1152 · 47** | **1199 · 1152 · 47** | memory 片段加在 user 消息（前缀之后）→ 前缀保留（96%） |
| streaming（flash） | **1183 · 1152 · 31** | **1183 · 1152 · 31** | SSE 最终 usage chunk 同样上报命中（97.4%） |
| identical（pro） | 1104 · 0 · 1104（冷） | **1104 · 1024 · 80** | Pro 同样缓存，热 **92.8%** |
| streaming（pro） | **1104 · 1024 · 80** | **1104 · 1024 · 80** | Pro streaming 92.8% |

### 关键结论

1. **稳定前缀是命中的决定性因素**：identical / last_msg_changed / memory_added 都
   命中 ~97%；改 system 一个字符从第 0 token 全失效（验证 prefix cache "只有相同前缀才能复用"）。
2. **动态尾部不破坏前缀**：最后 user 消息、追加 memory 片段都落在缓存前缀之后，命中保留。
3. **工具顺序/schema 变化破坏部分前缀**：命中率降到 73%/85%，二次请求后恢复 ~95%（代理重建缓存）。
4. **流式与非流式一致**：SSE 最终 usage chunk 上报同样命中率。
5. **Flash 与 Pro 都命中**：Pro 略低（92.8%），可能是不同路由/通道。

## 官方 DeepSeek（`https://api.deepseek.com`）—— 外部阻塞

当前环境无 `DEEPSEEK_API_KEY`，官方端点未测量。配置了 key 后重复同一命令即可：

```bash
BENCH_BASE_URL=https://api.deepseek.com/v1 \
BENCH_API_KEY=$DEEPSEEK_API_KEY \
BENCH_MODEL=deepseek-chat \
bash scripts/bench-deepseek-cache
```

## 复现

```bash
set -a; source ~/.config/aletheon/daemon.env; set +a
BENCH_MODEL=deepseek/deepseek-v4-flash bash scripts/bench-deepseek-cache
```

## 与本地 shape 的关系

- 本地 `InferencePrefixShape`（`executive::host::daemon::cache_shape`）是**诊断身份**，
  不决定命中；上面 identical 场景的 31 token miss（1183−1152）是 provider 侧
  `ProviderMissOrEviction`（冷缓存/逐出/代理行为），不是本地 shape 变化。
- 上述结果指导默认配置：`[providers.cache] reporting = "auto"`（字段自解释）在代理
  返回 DeepSeek 字段时已能正确计量；如需显式声明，可对 lejurobot 代理置
  `reporting = "deepseek_chat"`（见计划 C2）。
