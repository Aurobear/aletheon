# E7 extension protocol cutover — 2026-08-12

## Requirement and current-code anchors

CGP-02 assigns versioned commands, queries, events, serialization and legacy
translation to the independent Gateway protocol/client packages
(`docs/plans/2026-08-08-composition-gateway-presentation-extraction.md:313-320`).
E7 deletes extension-specific Fabric rich types and re-exports after caller-zero
(`docs/plans/2026-08-08-preserved-extensions-cutover.md:418-428`). The six
extension wire rows were registered for Gateway/Application ownership and E7
deletion (`config/architecture/fabric-boundary-census.tsv:408-413`).

## Result

The versioned lifecycle requests/receipt and MCP connector manifest/transport now
live in `crates/gateway-protocol/src/extension.rs`. Aletheon production wiring,
`aletheon-extension`, and inert Executive rollback code import that single
Gateway owner. Fabric's extension protocol module and its root declaration are
deleted.

The legacy `fabric::protocol::client::ClientRpcRequest` extension variants were
also deleted. The installed extension CLI already uses
`gateway_protocol::Command::ManageExtension`; keeping the legacy variants would
have retained a second published business-method vocabulary and forced Fabric to
depend back on Gateway protocol.

```text
Before: installed typed command + Fabric extension JSON-RPC DTO/variants
After:  gateway-protocol::Command::ManageExtension + versioned Gateway DTOs
        Fabric extension wire surface -> deleted
```

Caller-zero gates:

```bash
! rg -n 'fabric::protocol::extension|ClientRpcRequest::Extension' crates --glob '*.rs'
! test -e crates/gateway/src/protocol/extension.rs
```
