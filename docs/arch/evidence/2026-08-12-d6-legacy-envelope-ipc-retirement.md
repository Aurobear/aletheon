# D6 legacy Envelope and IPC message retirement

After removal of the legacy backends and communication primitives, exact workspace caller search showed the legacy `Envelope`, `Protocol`, `IpcMessage`, `IpcBackend`, low-level numeric IPC AgentId/Pid and agent-profile switch event family had no caller outside their own definitions/root exports. D6 therefore deleted:

- `ipc/envelope.rs`, `ipc/protocol.rs`, `ipc/ipc_msg.rs`, `ipc/ipc_types.rs`;
- `types/agent.rs` and the unused `types/agent_profile_event.rs`;
- all module and root compatibility re-exports.

The live typed `EnvelopeV2`, Runtime Agent identities/mailbox, Gateway protocol and provider-neutral audit paths remain unchanged. No compatibility alias was retained.
