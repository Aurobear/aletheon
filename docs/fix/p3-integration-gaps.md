# P3 — Integration Gaps

Status: **Open** | Priority: Medium

---

## P3.1 MCP Resources/Prompts Not Supported

- **File:** `crates/corpus/src/mcp/` (client)
- **Severity:** P3
- **Description:** The MCP client does not handle `resources/list`, `resources/read`, `prompts/list`, or `prompts/get`. Only tools and notifications are implemented.
- **Impact:** Agents cannot use MCP server resources or prompt templates. Limited MCP interoperability.
- **Fix direction:** Implement the four missing MCP method handlers in the client; add corresponding tool endpoints.

---

## P3.2 Discord/Slack/Email Delivery is info! Log Only

- **File:** `crates/corpus/src/notification/delivery.rs:47-55`
- **Severity:** P3
- **Description:** Notification delivery channels (Discord, Slack, Email) log `info!` messages but do not actually send anything to external services.
- **Impact:** No real notification delivery. Agents cannot alert users through external channels.
- **Fix direction:** Implement webhook-based delivery for Discord and Slack; add SMTP client for email.

---

## P3.3 io_uring IPC Recv Path Incomplete

- **File:** `crates/fabric/src/ipc/` (io_uring backend)
- **Severity:** P3
- **Description:** Only setup and write paths work; the receive/read path is not implemented. IPC is effectively write-only over io_uring.
- **Impact:** High-performance IPC cannot be used for bidirectional communication.
- **Fix direction:** Complete the io_uring recv submission/completion handling; add integration tests with real ring operations.

---

## P3.4 9 SystemClock Calls in Corpus Not in CI Enforcement Scope

- **File:** `crates/corpus/src/` (various)
- **Severity:** P3
- **Description:** 9 direct `SystemClock` calls exist in the corpus crate but are not covered by the architecture fitness CI check that enforces `WallTime` usage elsewhere.
- **Impact:** Time dependency not injectable in these locations; tests may be time-sensitive.
- **Fix direction:** Add corpus to the CI architecture check scope; migrate the 9 calls to `WallTime`.

---

## P3.5 2 Direct Tool::execute Calls in Dasein Not in CI Enforcement Scope

- **File:** `crates/dasein/src/` (various)
- **Severity:** P3
- **Description:** 2 direct `Tool::execute` calls in the dasein crate bypass the tool execution framework, not covered by CI architecture enforcement.
- **Impact:** Tool execution in dasein is not sandboxed or monitored; architecture drift.
- **Fix direction:** Add dasein to the CI architecture check scope; route through the standard tool execution pipeline.
