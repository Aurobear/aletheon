# D6 Cognit domain owner cutover

The former shared `contracts::cognit` module contained Cognit planning, critique,
reflection, learning, observation, and evolution aggregates rather than ownerless
value primitives. D6 moved the definitions intact to `cognit::domain`, updated
Cognit and downstream Mnemosyne/Metacog/Interact/Executive/Aletheon callers, and
removed the shared module and flat compatibility re-exports.

`WorkspaceContent::Plan` had no constructor or payload consumer outside the
shared definition; retaining it would force Contracts to depend on a Cognit-owned
aggregate. It was therefore retired rather than replaced with a second schema.

Validation:

```text
bash scripts/cargo-agent.sh check -p cognit --all-targets      PASS
bash scripts/cargo-agent.sh check -p mnemosyne --all-targets  PASS
bash scripts/cargo-agent.sh check -p metacog --all-targets    PASS
bash scripts/cargo-agent.sh check -p interact --all-targets   PASS
bash scripts/cargo-agent.sh check -p executive --all-targets  PASS
bash scripts/cargo-agent.sh check -p aletheon --tests         PASS
```
