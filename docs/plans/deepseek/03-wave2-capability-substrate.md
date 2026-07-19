# Wave 2：能力底座

**状态：** 被阻塞。**解锁条件：** W1-06 与 H1-08 均通过。
**上游：** `docs/plans/2026-07-19-wave2-capability-substrate.md`。

## 唯一顺序与硬裁决

| ID | 实现 | 固定决策 | 验证 |
|---|---|---|---|
| W2-01 | `ToolResultMeta` 墠加 hash/lines/artifact/truncated 字段 | 字段为 serde default 的 `Option` | `bash scripts/cargo-agent.sh test -p fabric tool_result_meta` |
| W2-02 | `file_read` 计算 SHA-256 与 total_lines | hash 对原始字节计算；空文件行数为 0 | `bash scripts/cargo-agent.sh test -p corpus file_read` |
| W2-03 | `file_write` expected_sha256 | 文件存在时该字段必填；不匹配返回 `StaleWorkspaceView` | `bash scripts/cargo-agent.sh test -p corpus file_write` |
| W2-04 | `apply_patch` expected_sha256 与 checkpoint | 一次请求只允许一个文件；多文件输入 schema 拒绝 | `bash scripts/cargo-agent.sh test -p corpus apply_patch` |
| W2-05 | `glob` cursor/limit/排序 | cursor 为最后路径的 URL-safe base64；排除集固定 `.git,target,node_modules` | `bash scripts/cargo-agent.sh test -p corpus glob` |
| W2-06 | 内容寻址 Artifact Store 与 `artifact_read` | data_dir 下按 SHA-256 分层；分页单位为字节 | `bash scripts/cargo-agent.sh test -p corpus artifact` |
| W2-07 | MCP config 移入 corpus provider | corpus 对 cognit 依赖归零 | `bash scripts/cargo-agent.sh build -p corpus` |
| W2-08 | CredentialPort | trait 归 `platform-api/security.rs`；Executive 注入实现 | `bash scripts/cargo-agent.sh test -p corpus mcp_auth` |
| W2-09 | exec-server 去 corpus | structured_patch 固定来自 platform-api | `bash scripts/cargo-agent.sh build -p exec-server` |
| W2-10 | 创建 `runtime-api` | 文件固定 manifest/work_order/lifecycle/events/receipt/transport | `bash scripts/cargo-agent.sh test -p runtime-api` |
| W2-11 | 创建 `runtime-broker` | selector 顺序：Named → capability → health → workspace → policy；无匹配直接拒绝 | `bash scripts/cargo-agent.sh test -p runtime-broker` |
| W2-12 | native adapter 与删除 Pi 特判 | 创建 runtime-native-cognit；删固定 ID 与 `.contains("pi")` | `bash scripts/cargo-agent.sh test -p executive runtime_contract` |
| W2-13 | 收紧架构门禁 | corpus→cognit/mnemosyne、exec-server→corpus 均为 0 | `bash tests/architecture_check.sh` |

## 禁止事项

- 禁止 fallback 到不健康 runtime。
- 禁止无 expected_sha256 覆盖已存在文件。
- 禁止 Artifact Store 返回未校验的相对路径。
- 禁止 Executive 或 Goal import Pi 具体类型。
- 每个 ID 单独提交；W2-01 到 W2-13 严格串行。
