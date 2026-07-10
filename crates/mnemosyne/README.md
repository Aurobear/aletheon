# memory

Memory subsystem for the Aletheon agent.

## Overview

Provides persistent memory storage with multiple backends and memory operations.

## Architecture

```
backends/
├── episodic/    — Temporal events and reflections
├── semantic/    — Knowledge with FTS5 + vector search
├── procedural/  — Learned procedures and skills
└── self_memory/ — Self-awareness records

ops/
├── router.rs        — Memory routing by type
├── consolidation.rs — Memory consolidation
├── decay.rs         — Memory decay/forgetting
├── activation.rs    — Memory activation
└── schema.rs        — Memory schema definitions
```

## Key Types

- `EpisodicMemory` — Temporal event storage
- `SemanticMemory` — Knowledge with vector search
- `ProceduralMemory` — Skill storage
- `MemoryRouter` — Routes memories to appropriate backends
