# D6 Mnemosyne memory domain cutover

D6 split the old mixed shared memory module by ownership. Memory entries,
queries, filters, handles, statistics, compaction models and the backend port
are Mnemosyne domain concepts and now live at `mnemosyne::memory`. All backend,
router and test callers consume the owner directly.

The dependency-neutral embedding provider and machine/provider backpressure
coordination contracts remain in Contracts because both Cognit inference and
Mnemosyne embeddings consume the same machine-scoped admission boundary; moving
them to Mnemosyne would introduce the forbidden Cognit/Mnemosyne dependency
cycle.

Validation:

```text
bash scripts/cargo-agent.sh check -p mnemosyne --all-targets  PASS
bash scripts/cargo-agent.sh check -p executive --all-targets  PASS
bash scripts/cargo-agent.sh check -p aletheon --tests         PASS
```
