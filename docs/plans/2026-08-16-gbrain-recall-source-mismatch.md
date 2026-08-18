# GBrain 记忆召回失效（Codex Handoff）

状态:`ACTIVE — 待 Codex 执行`  
日期:2026-08-16  
分支:`fix/tui-live-agent-inspector`  
基线:当前工作区（未提交迁移路径很多，禁止 `git add -A`）  
前置:迁移收尾已落地；本文件只聚焦 gbrain 记忆召回，不触碰架构耦合包

> 本文档是**根因交接 + 待办清单**，不是新架构设计。所有 locator 已在本会话核实到当前工作区代码。
> 核心待办在问题 B（aletheon 侧 source 错位）；问题 A 是 aurb 侧已修的上下文。

---

## 症状

用户反馈：gbrain 用不了，codex / grok 都拿不到历史记忆。

实际验证：`/usr/bin/aletheon memory recall` 对任何查询都返回空 items，且 `degraded_sources` 为空（即"没报错、也没召回"）：

```json
{"degraded_sources": [], "items": [], "request_id": "..."}
```

## 已核实的健康基线（2026-08-16）

- gbrain MCP server 正常：`gbrain-mcp.service` active，绑定 `100.120.122.46:3131`，`search`/`query` 工具能命中数据
- token 注入正常：`bash -lic 'echo $GBRAIN_READ_TOKEN'` 能拿到 token（`.bashrc → provider-env.sh → ~/.config/aurb/gbrain.env` 链路通）
- aletheon daemon 正常：`aletheon.service` active（PID 800759），启动时 4 个 gbrain MCP server（`gbrain-aletheon/aurb/gbrain/personal`）各发现 96 工具
- 端点通：`127.0.0.1:3131/mcp` 与 `100.120.122.46:3131/mcp` 均可达（401 等待 token）
- **唯一坏点：`aletheon memory recall` 恒空**

## 问题总览

| # | 严重度 | 问题 | 位置 | 状态 |
|---|---|---|---|---|
| A | 🟡 | recall hook 超时太短（1.0s < recall 1.7s），codex 侧静默不注入 | `src/hooks/recall.sh:24`（aurb 仓库） | ✅ 已修（1.0→3.0），待 `aurb.sh sync` |
| B | 🔴 | aletheon recall 恒空：写/读 source 错位 + 孤立 chunk | `crates/aletheon/src/wiring/adapters/gbrain/mcp_adapter.rs` | ⬜ 待修 |

---

## 问题 A：recall hook 超时太短（aurb 侧，已修）

`aletheon memory recall` 实测耗时 0.6s–1.7s 波动。aurb 的 recall hook 默认 timeout 是 1.0s：

```python
# src/hooks/recall.sh:24（aurb 仓库）
timeout = max(0.1, min(float(os.environ.get("ALETHEON_MEMORY_RECALL_TIMEOUT_SECONDS", "1.0")), 5.0))
```

慢时超过 1s → `subprocess.TimeoutExpired` → hook 的 `except Exception: sys.exit(0)` 静默吞掉 → codex 一个字都不注入。已改为默认 3.0s。**此问题在 aurb 仓库，非本仓库；本仓库无需处理，仅记录为上下文。**

---

## 问题 B：aletheon recall 恒空 —— source 错位（核心，待修）

### 根因链

```
observable data（gbrain 实际数据）
   ├─ default 源：可召回页面（leju-joystick 手柄页、aletheon/goal_outcome/*）
   ├─ aletheon 源：aletheon/semantic_fact/* 孤立 chunk（query 命中，get_page 报 page_not_found）
   └─ aurb/gbrain/personal 源：少量页面

aletheon recall 读路径（workspace 绑定）
   expected_read_sources = ["aletheon","personal"]      ← 绑定写死
        │
        ▼
   mcp_adapter query(source_id="aletheon")  → 命中孤立 semantic_fact chunk
        │
        ▼
   get_page(slug)  → page_not_found → is_supplemental_memory_document 过滤 → continue
        │
        ▼
   命中全被丢弃 → items 空；local memory 又只有 goal-scope 记录 → 整体空
```

### 事实（已核实）

1. **gbrain 数据源分布**：`gbrain local-status` 显示 84 页，5 个 source（`aletheon/aurb/default/gbrain/personal`），84 页**全是 orphan + stale**，`brain_score=45`。用 `GBRAIN_READ_TOKEN` 调 `list_pages` 只见 77 页全 `source_id: default`。
2. **写路径不传 source**：`crates/aletheon/src/wiring/adapters/gbrain/mcp_adapter.rs:428-446` 的 `put_page` 只发 `{slug, content}`，不传 `source_id`。gbrain 的 `put_page` 工具参数表里也没有 source_id（`source_kind`/`source_uri` 标注 "SERVER-STAMPED, client value ignored"）—— 页面源由 OAuth client 的 source_id 决定。
3. **读路径按绑定 source 过滤**：`mcp_adapter.rs:448-470` 的 `query` 发 `{query, source_id, limit}`；`mcp_adapter.rs:171-312` 的 `SupplementalBindingRecallPort::recall` 迭代 `binding.expected_read_sources`（`mcp_adapter.rs:192-195` 的 `read_destination_handles` zip `expected_read_sources`）。
4. **绑定期望 source**：`~/.local/state/aletheon/memory-gateway/workspace-bindings-v1.db` 表 `workspace_memory_bindings`，aurb 仓库绑定 `expected_read_sources_json=["aletheon","personal"]`、`expected_write_source="aletheon"`、`state="active"`。
5. **孤立 chunk 被过滤**：`aletheon` 源里 `aletheon/semantic_fact/*` 的 `query` 命中，但 `get_page` 返回 `page_not_found`；`is_supplemental_memory_document`（`mcp_adapter.rs:854-867`）要求 `schema == PAGE_SCHEMA_VERSION`，非目标 schema 直接 `continue`（`mcp_adapter.rs:251-258`）。命中全被丢弃后 `degraded` 仍为 false → `degraded_sources` 空。
6. **local 侧也空**：`~/.local/state/aletheon/memory_consolidation.db` 的 `memory_records` 4620 条全是 `scope_json={"kind":"goal","id":"agent:…"}`（goal-scope）+ `kind="goal_outcome"`，而 recall prefilter 按 workspace + session scope（`crates/mnemosyne/src/memory_gateway.rs:273-379` 的 `RecallPreFilter`），匹配不到 goal-scope 记录。

### 根因（一句话）

**写路径 `put_page` 不传 source（页面落到 OAuth client 的默认源 `default`），读路径 `query` 却按绑定的 `expected_read_sources=["aletheon","personal"]` 过滤，读写 source 不一致；且 `aletheon` 源里只有孤立 semantic_fact chunk（get_page 失败），导致 recall 命中全被丢弃。**

### 待核实（Codex 排查点）

1. gbrain 的 OAuth client `aletheon`（`GBRAIN_ALETHEON_CLIENT_ID`）对应的 source_id 到底是什么？为什么 `goal_outcome` 页面落在 `default` 而不是 `aletheon`？（历史数据由旧 client 写？还是 client→source 映射漂移？）
2. `aletheon/semantic_fact/*` 孤立 chunk 是怎么产生的：embedding 索引里存在、对应页面却 `page_not_found`，是 projection 半途失败还是软删除后未清理索引？
3. `expected_read_sources` 的 `["aletheon","personal"]` 是谁、在哪一步写进绑定的？（`aletheon memory workspace bind` 的调用方，或 bootstrap 默认值）

### 修复方向（三选一，需定夺后实施）

- **方向 1（读改绑）**：把 workspace 绑定的 `expected_read_sources` 改成实际数据所在源（如 `default`/`aletheon`），让 recall 读到现有页面。最快，但会把非 aletheon 页面（如手柄页）也纳入召回。
- **方向 2（写对齐 + 重投影）**：让 `put_page` 显式写入 `aletheon` 源（或修正 client→source 映射），并重投影历史数据，使读写都落 `aletheon/personal`。最干净，但要数据迁移 + 重投影。
- **方向 3（适配层兜底）**：`query` 命中 `page_not_found` 时改走 `search`（无 source_id 过滤，`mcp_adapter.rs:472-483`）并回退到 `expected_read_sources` 交集；或对 `is_supplemental_memory_document` 失败降级。治标，不治源。

### 验证

```bash
# 症状复现（期望 items 非空，实际恒空）
/usr/bin/aletheon memory recall --working-dir "$(pwd -P)" --max-items 10 '手柄'

# 绑定核实
sqlite3 ~/.local/state/aletheon/memory-gateway/workspace-bindings-v1.db \
  "SELECT workspace_key, expected_read_sources_json, expected_write_source, state
   FROM workspace_memory_bindings WHERE state='active';"

# 数据源核实（gbrain MCP；需 GBRAIN_READ_TOKEN）
#   query source_id=default  '手柄'  → 命中 leju-joystick 页
#   query source_id=aletheon '手柄'  → 命中 aletheon/semantic_fact/*（get_page 报 page_not_found）
#   get_page aletheon/semantic_fact/… → {"error":"page_not_found"}

# local 记录核实
sqlite3 ~/.local/state/aletheon/memory_consolidation.db \
  "SELECT json_extract(scope_json,'$.kind'), count(*) FROM memory_records GROUP BY 1;"
```

---

## 相关环境

- gbrain server：`~/.bun/bin/gbrain serve --http --port 3131 --bind 100.120.122.46`（version 0.42.70.0）
- SSH tunnel：`gbrain-ssh-tunnel.service`（`ssh -L 127.0.0.1:3131:100.120.122.46:3131 aurobear@aurobear-ser8`）
- connector（aurb 仓库）：`src/aletheon/connectors/gbrain.json`（`url=http://127.0.0.1:3131/mcp`，`bearer_token_env=GBRAIN_READ_TOKEN`，`allowed_tools=["search","get_page"]`，`request_timeout_ms=2000`）
- 凭证：`~/.config/aletheon/daemon.env`（`GBRAIN_{ALETHEON,AURB,GBRAIN,PERSONAL}_CLIENT_ID/SECRET` + `GBRAIN_READ_TOKEN`）
- gbrain 数据目录：`~/.gbrain/brain.pglite/`

## 边界提醒

- gbrain 仓库（`agent/gbrain`）当前 clean，属外部 MCP/OAuth 服务，**不在本次修复范围**（按 aurb `src/skills/general/gbrain-memory/references/unified-memory.md` 的权威边界，source binding 归 aletheon）。
- 本仓库工作区有大量未提交迁移路径，提交由用户授权后按包执行，禁止 `git add -A`。
