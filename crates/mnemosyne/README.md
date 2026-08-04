# mnemosyne

Persistent memory services, recall, consolidation, retention, projections, and
knowledge-graph operations for Aletheon.

## Current ownership

- `service/` and `fact_service` expose request-facing memory use cases.
- `recall/`, `projection/`, and `retention/` implement governed retrieval and
  lifecycle policies.
- `runtime` exposes composition-only local SQLite-backed handles.
- `supplemental` owns product-neutral remote-memory contracts and the durable
  local outbox; the host supplies remote transport.
- `cognitive-memory`, vector backends, and LLM synthesis are off-by-default or
  experimental features, not installed-runtime defaults.

Concrete storage handles remain behind stable `MemoryService` and fact-use-case
contracts so application code does not depend on adapters.

## Validation

```bash
bash scripts/cargo-agent.sh test -p mnemosyne
```
