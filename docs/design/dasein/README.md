# Dasein Crate — Self and Lived-State Policy

`dasein` owns identity, boundary, care, narrative, continuity, lived
temporality, mutation transitions, and host-facing perception policy.

## Current structure

```text
crates/dasein/src/
  core/         SelfField policy/read-model layers
  dasein/       versioned lived-state reducer and persistence
  bridge/       loop-detector and policy adapters
  impl/
    perception/ perception manager, sources, and experimental AgentFs/FUSE stub
    mutation/   mutation approval policy
  testing/      mock perception support
```

Hooks are owned by Corpus/Executive integration, while admission, resource
governance, supervision, audit, and sandbox enforcement belong to Kernel,
Executive, Fabric, and Corpus. Removed self-protection/resilience classes are
retained only in explicitly historical documents.

## Documents

| Document | Status |
|----------|--------|
| [self-field.md](self-field.md) | Current conceptual layers and ownership boundaries |
| [perception.md](perception.md) | Current perception design with experimental-source caveats |
| [first-principle.md](first-principle.md) | SelfField design principle |
| [writable-root.md](writable-root.md) | Current sandbox path-isolation design |
| [self-protection.md](self-protection.md) | Historical removed implementation design |
| [resilience.md](resilience.md) | Historical removed implementation design |
