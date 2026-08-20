# contracts

Shared cross-domain contracts, identifiers, events, protocol types, and
compatibility infrastructure for Aletheon.

## Ownership boundary

`contracts` defines stable data and port contracts. Runtime policy, host I/O,
provider implementations, and clock implementations belong to their owning
crates. In particular, the `Clock` contract is exported by `contracts`, while
`SystemClock` and `TestClock` are implemented by `kernel::chronos`.

## Current layout

```text
src/
  contract/    cross-domain contracts
  include/     subsystem and host-facing ports
  types/       shared domain and wire values
  protocol/    client/runtime protocol schemas
  events/      event contracts
  ipc/         IPC envelopes and compatibility transports
  policy/      shared policy contracts
  primitives/  low-level identifiers and value types
  dasein/      self-domain contracts
  adapters/    contracts-side adapters
  + root-level compaction / reflection / turn-policy modules
```

The root facade retains compatibility re-exports, but new implementation
containers must not be added there. Kernel lifecycle, process, admission, and
time implementations live in the separate `kernel` crate.

## Validation

```bash
bash scripts/cargo-agent.sh test -p contracts
bash scripts/aletheon.sh acceptance architecture
```
