# cognit

Focused cognitive algorithms and harnesses for reasoning, planning, critique,
reflection, learning, and model inference.

## Current layout

```text
src/
  core/         focused cognitive algorithms; no production aggregate owner
  harness/      production cognitive sessions, including the linear harness
  application/  inference and learning use cases
  adapters/     private provider and policy adapters
  bridge/       narrow integration bridges
  composition/  inference construction
  ports/        stable input/output ports
```

Production composition belongs to the harness/session boundary. The former
`CognitCore` aggregate is not a production entry point. Provider transports are
private adapters; consumers use the inference contracts and scheduler facade.

The rule-based `IntentClassifier` and `InferenceRouter` exist as application
components, but the installed runtime currently selects providers through the
host-owned inference boundary rather than an offline-first local-model route.

## Validation

```bash
bash scripts/cargo-agent.sh test -p cognit
```
