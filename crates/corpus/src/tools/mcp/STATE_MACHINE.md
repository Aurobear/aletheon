# MCP client and authorization state machines

`lifecycle.rs` owns connection and OAuth transition policy. Server health is mutated
only by `apply_server_transition`; OAuth is mutated only by
`McpOAuthLifecycle::apply` inside the provider.

```text
connection: absent -> Connecting -> Connected <-> Reconnecting -> Degraded -> Stopped
OAuth: Unauthenticated -> Authorizing -> Exchanging -> Authorized <-> Refreshing
                                           \-> Failed
```

Connection effects cover initialization, discovery generation, request routing,
notification supervision, reconnect, and shutdown. Every background notification,
health, and reconnect task is registered with `McpTaskSupervisor`; shutdown cancels,
joins, then aborts only at the bounded deadline. Elicitation remains fail-closed when
no approval handler is present.

Endpoint policy approval precedes credential resolution. Credential grants compare
the exact normalized endpoint and expiry; discovery rejects redirects and unsafe
addresses. OAuth CSRF state is single-consume and time bounded. Token store writes are
atomic persistence points; pending CSRF state and connection tasks are intentionally
ephemeral. Repeated callback, token-store, and post-shutdown events fail closed.
Restart reloads durable tokens, while discovery and connections restart from their
initial states. Frame/request sizes and transport timeouts remain enforced by the
transport port.
