# D6 unused legacy IPC/backend retirement

The D0 ledger assigned the old Fabric IPC backends, transport implementations and legacy communication protocols to D6 deletion after installed/config/route caller-zero evidence (`docs/plans/2026-08-09-fabric-source-disposition-ledger.md:76-99,445-479`).

Evidence before deletion:

- workspace Rust caller search found no caller outside the legacy IPC subtree for `IpcManager`, `IpcBackendAdapter`, `IoUringBackend`, `JsonRpcAdapter`, `SharedMemBackend`, `UnixSocketBackend`, `PriorityQueue`, `InProcessTransport`, `PubSubProtocol`, or `RequestResponseProtocol`;
- the current installed deployment uses the typed Gateway client/server and official Unix socket, not these Fabric legacy backends;
- the prior L3 installed runtime, official client request, Memory Agent smoke and stable service counters passed;
- no configuration/route surface selects these legacy types.

Deleted:

- `fabric/src/ipc/backends/**`;
- `fabric/src/ipc/transport/**`;
- legacy `CommunicationBus`, `InProcessTransport`, pub/sub and request/response implementations;
- their legacy-only `protocol_e2e` test.

The live schema-filtered `CanonicalEventBus` is intentionally retained pending its Runtime/owner cutover; this deletion does not replace it with a second bus.
