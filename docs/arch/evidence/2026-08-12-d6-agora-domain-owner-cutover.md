# D6 Agora domain owner cutover

The transactional workspace operation, proposal, commit, permit, view and service
interfaces were rich Agora state and authority contracts. D6 moved the complete
family from `contracts::include::agora` to `agora::contract`, updated Agora,
Corpus, Executive and Aletheon callers to depend on the owner crate, and removed
all shared-root and flat compatibility exports.

Validation:

```text
bash scripts/cargo-agent.sh check -p agora --all-targets      PASS
bash scripts/cargo-agent.sh check -p corpus --all-targets     PASS
bash scripts/cargo-agent.sh check -p executive --all-targets  PASS
bash scripts/cargo-agent.sh check -p aletheon --tests         PASS
```
