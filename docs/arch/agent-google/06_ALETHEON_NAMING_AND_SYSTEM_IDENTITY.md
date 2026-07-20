# Aletheon Naming and System Identity

> **Status:** Accepted system identity and naming policy
>
> **Verified snapshot:** 2026-07-19

## 1. Name Origin

`Aletheon` is best treated as a constructed project name inspired by the philosophical concept of `Aletheia`.

Aletheia is associated with:

- truth;
- disclosure;
- unconcealment;
- unhiddenness;
- revealing what was previously concealed.

`Aletheon` should not be claimed as a strict standard Ancient Greek word without specialist verification. It is a modern project name derived from that philosophical root.

## 2. Formal Meaning

English:

> **Aletheon is a persistent Agent system through which intention, memory, world, and action are brought into disclosure and transformed into sustained execution.**

Chinese:

> **Aletheon 是一个让意图、记忆、世界与行动从遮蔽中显现，并持续转化为现实执行的 Agent 系统。**

Engineering definition:

> **Aletheon is a native-first Agent runtime that maintains identity, active cognition, goals, experience, and supervised action across time.**

## 3. Module Meaning

```text
Aletheon
    The complete user-facing system and application entry.

Executive
    How intention becomes scheduled, bounded, supervised and finally verified.

Kernel
    The unavoidable lifecycle and governance primitives: operation, process,
    admission, time, space and supervision.

Runtime
    How an external task executor declares capabilities, receives a WorkOrder,
    maintains a session and returns events and an untrusted receipt.

Cognit
    How the system understands, reasons, plans and reviews.

Dasein
    Who exists, chooses, commits and continues.

Agora
    Where active goals, observations, hypotheses and conflicts appear together.

Mnemosyne
    How experience is retained, recalled, consolidated and transformed into knowledge.

Metacog
    How candidate changes are evaluated and evolved under governance.

Corpus
    How governed tools and external capabilities are discovered and invoked.

Platform
    How Aletheon accesses host operating-system capabilities across Linux,
    Windows and macOS.

execd
    How approved low-level file and process side effects run in an isolated daemon.

Hardware
    How governed device identity, leases, commands, telemetry and safety semantics
    connect Aletheon to physical systems.

Fabric
    The shared protocol, identity and envelope vocabulary between domains.

Interact
    How users communicate with and observe the system.

Gateway
    How external request protocols enter the system without owning domain state.
```

### 3.1 Boundary Vocabulary

The names are not interchangeable:

```text
Executive  = system orchestration and final decision
Kernel     = invariant lifecycle/governance mechanism
Runtime    = supervised external task executor
Platform   = host OS capability library
execd      = isolated low-level side-effect process
Hardware   = physical-device control domain
Corpus     = governed tool and capability execution
```

A Runtime may request capabilities, but it cannot grant itself permission or
verify global task completion. Platform exposes host mechanics but does not own
Agent policy. Hardware owns device semantics but not the hard real-time edge
control loop.

## 4. System Identity

Aletheon is not only:

```text
an AI assistant
a chatbot
a coding agent
a memory database
a workflow engine
```

A better description is:

> **Aletheon is a personal Agent system with native cognition, persistent goals, supervised subagents, external capabilities, and an experience architecture.**

## 5. External Components

External products are adapters or providers, never identity owners:

```text
Coding runtimes
    Supervised task executors selected through Runtime contracts.

Model providers
    Inference backends selected by policy; no provider owns Aletheon identity.

Knowledge backends
    Storage/search providers behind Mnemosyne contracts.

Messaging and mail channels
    Input/output adapters behind Gateway and Interact boundaries.

Robot edge runtimes
    Hard-real-time and device-local safety authorities behind Hardware providers.
```

Aletheon retains primary cognition, identity, goal continuity, lifecycle,
permissions, resource governance, memory policy, subagent coordination and
embodiment regardless of which external provider is configured.

### 5.1 Crate Naming Policy

Production crate names must:

- use one lowercase word without hyphens;
- name a stable domain owner, not a temporary layer such as `api`, `types`,
  `common`, `broker` or `utils`;
- have a real production caller before being declared production-ready;
- remain an internal module unless independent compilation, dependency
  isolation, deployment or security boundaries justify a crate.

Current production and explicitly experimental domain crates are:

```text
agora aletheon cognit corpus dasein execd executive fabric gateway hardware
interact kernel metacog mnemosyne platform runtime
```

Examples use descriptive snake_case package names and do not define production
architecture.

## 6. Tagline

Primary:

> **Aletheon — From intention to sustained action.**

Alternative:

> **Aletheon — A persistent Agent system for memory, goals, and action.**

Philosophical:

> **Aletheon — Where intention, memory, and world come into disclosure.**
