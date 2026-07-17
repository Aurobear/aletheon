# ADR: retain Fabric protocol and extension contracts in the workspace

- Status: accepted (no physical split for V02)
- Date: 2026-07-16
- Decision owner: release architecture gate

## Context

The source requirement permits extracting an application protocol, Fabric
transport/runtime implementations, or an extension SDK only when measured
edges or an external ABI need justify it. It explicitly rejects splitting to
reduce line count.

Current workspace manifests show Fabric as the shared contract dependency of
Agora, Cognit, Corpus, Dasein, Executive, Interact, Kernel, Metacog, and
Mnemosyne. Fabric exposes protocol and domain contract modules together, while
no checked-in manifest, published package configuration, ABI compatibility
suite, or out-of-tree plugin build establishes an external stable-ABI consumer.
The V02 release gate records `cargo tree --workspace --edges normal` as the
reviewable dependency artifact after all acceptance lanes pass.

## Decision

Do not physically split `aletheon-app-protocol`, Fabric transport/runtime, or an
extension SDK in V02. A new crate would move existing edges rather than remove a
verified dependency edge, and there is no evidenced external ABI to stabilize.
Keep the logical protocol and extension boundaries in their current modules and
measure them in each release acceptance bundle.

## Reconsideration threshold

Reopen this ADR only when at least one condition is demonstrated by artifacts:

1. `cargo tree` plus an architecture test shows extraction removes a forbidden
   normal dependency edge rather than renaming it;
2. an out-of-tree consumer requires a semver-governed protocol/SDK surface and
   passes an ABI/API compatibility suite; or
3. transport runtime code prevents a contract-only consumer from building
   without runtime dependencies, with measured build or deployment impact.

Any proposal must include before/after dependency trees, consumer build tests,
compatibility policy, migration plan, and ownership. Line count alone is not
evidence.

## Consequences

V02 adds no crate churn or compatibility promise that the project cannot yet
test. Fabric remains a broad internal contract crate, so the architecture gate
must continue recording dependency edges; a future split remains optional and
evidence-driven.
