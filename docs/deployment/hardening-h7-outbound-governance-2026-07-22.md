# H7 Outbound Governance Evidence — 2026-07-22

## Requirement anchors

- 在现有 crate 内建立最小 endpoint/policy/transport 边界，先迁移 MCP/Google，保留协议 retry，
  并约束 DNS、redirect、timeout：`docs/plans/2026-07-21-production-readiness-hardening.md:232-240`。
- 默认 deny 不回归；loopback 只能由显式 trust class 放行；credential 必须在 endpoint 获批后
  解析：`docs/plans/2026-07-21-production-readiness-hardening.md:242-246`。

## Implemented boundary

```text
Google / MCP protocol adapter
           |
           v
EndpointPolicy -- identity + trust class + timeout/redirect rules
           |
           +-- initial DNS approval
           +-- reqwest resolver revalidation on connection
```

- Corpus 内最小 policy/transport helper 不知道 Google/MCP 业务协议，也没有形成万能 service
  （`crates/corpus/src/tools/outbound.rs:1-12,45-176`）。
- Public Internet 拒绝 loopback、link-local/metadata、RFC1918、CGNAT、IPv6 ULA/loopback；
  `LocalLoopback` 只允许 loopback；`TrustedPrivateNetwork` 允许远程私网但仍拒绝 loopback、
  link-local/metadata（`crates/corpus/src/tools/outbound.rs:14-19,137-145,179-225`）。
- client 禁止自动 redirect、connect timeout 最大 10 秒、总 timeout 最大 30 秒，并在每次实际
  DNS resolution 后复验全部地址（`crates/corpus/src/tools/outbound.rs:99-176`）。因此 DNS
  rebinding 到任一禁止地址会由 resolver fail closed；redirect 不会携带 credential 到下一跳。
- MCP 在构造 bearer/OAuth 之前批准 endpoint，`LocalTrusted`/`RemoteTrusted`/`Untrusted`
  分别映射 loopback/可信远程网络/public-only（`crates/corpus/src/tools/mcp/client.rs:49-134`）。
  OAuth discovery、authorization、token endpoints 均在读取 client id/secret 环境变量前批准
  （`crates/corpus/src/tools/mcp/client.rs:152-202`）。
- Google API 请求在 `access_token` 调用前验证配置 authority 与解析结果
  （`crates/corpus/src/tools/google/client.rs:137-145,257-278`）；Google OAuth 在读取/发送 token、
  refresh token 或 client secret 前批准对应 endpoint
  （`crates/corpus/src/tools/google/oauth.rs:128-236`）。
- 本地与远程 GBrain 配置必须显式声明 trust；示例位于
  `docs/deployment/README.md:65-88` 与 `config/aletheon.example.toml:64-70`。

## Candidate assessment

| Candidate | Current evidence | H7 decision |
|---|---|---|
| Telegram | Gateway-owned client，token 位于 URL path（`crates/gateway/src/telegram/mod.rs:22-80`） | 下一次跨 crate port 的首要候选；不复制 Corpus 私有实现 |
| Automation | parked/future，多个 delivery channel 是 placeholder（`crates/executive/src/impl/automation/delivery.rs:9-16,29-88`） | 未启用路径，不在 H7 扩 scope |
| Qdrant | feature-gated client（`crates/mnemosyne/src/impl/vector_store.rs:142-163`） | 启用该 feature 前迁移 |
| LLM | provider adapter 与 scheduler 拥有既有 retry 语义（`crates/cognit/src/impl/llm/scheduler.rs:31-74`） | 不在 H7 改写 provider retry |

## Deterministic validation

```text
bash scripts/cargo-agent.sh test -p corpus tools::outbound::tests --lib
# 6 passed

bash scripts/cargo-agent.sh test -p corpus oauth_selection_tests --lib
# 4 passed

bash scripts/cargo-agent.sh test -p corpus tools::mcp::manager::tests --lib
# 19 passed

bash scripts/cargo-agent.sh test -p corpus tools::google::oauth::tests --lib
# 11 passed

bash scripts/cargo-agent.sh test -p corpus \
  --test google_read_only --test google_delta_sync --test gmail_history_sync
# 15 passed

bash scripts/cargo-agent.sh test -p corpus --lib
# 461 passed

bash scripts/architecture-check.sh
# 28 findings, 36 dependencies, 4 paths; no additions
```

覆盖 reserved address、DNS 后复验、redirect 不跟随、显式 loopback/private trust、credential
解析顺序，以及 MCP/Google 原有 retry/认证/健康回归。受控真实外部 endpoint smoke 留给最终 SER8。
