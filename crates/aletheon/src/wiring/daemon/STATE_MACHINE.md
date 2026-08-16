# Daemon server state machines

The daemon has one listener supervisor and one protocol lifecycle per authenticated
connection. `protocol.rs` owns negotiation; `ConnectionProtocolState::apply` is its
sole mutation entry and delegates policy to a pure reducer.

```text
listener: bind/adopt -> accept -> authenticate -> supervise clients -> drain -> stopped
client: New -> AwaitingInitialized -> Ready(versioned|legacy) -> disconnected
```

The host accepts only an owned Unix listener, including validated single-descriptor
systemd activation. Peer credentials are checked before a connection context exists.
Version negotiation rejects unsupported, missing, repeated, out-of-order, and mixed
legacy/versioned events. Only `Ready` connections dispatch application ports.
Subscription and request children stay in connection-owned `JoinSet`s; global client
tasks stay in the server-owned `JoinSet` and are cancelled/drained on shutdown.

Frame length and notification queue limits bound memory. Socket directory and socket
permissions are applied before serving. Connection state is intentionally ephemeral:
disconnect or crash requires authentication and negotiation again, so there is no
persistence or replay key. JSON-RPC request IDs and application-level operation IDs
provide request idempotency. Listener cancellation stops admission and gives existing
children a bounded graceful drain; late events are rejected with the closed channel.
