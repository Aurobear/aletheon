# Mnemosyne Crate — Backend Memory Storage

> Code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

**Crate:** `mnemosyne`
**Purpose:** Backend memory storage implementations. Provides episodic, semantic, procedural, and self-memory backends, plus a router that dispatches queries to the appropriate backend.

---

## Internal Structure

```
mnemosyne/src/
  lib.rs                      # Crate root
  schema.rs                   # Schema definitions for memory storage
  router.rs                   # MemoryRouter — routes queries to backends
  episodic.rs                 # Episodic memory (time-ordered experiences)
  semantic.rs                 # Semantic memory (knowledge graph / embeddings)
  procedural.rs               # Procedural memory (learned skills / patterns)
  self_memory.rs              # Self-memory (self-model / introspection)
  testing/                    # Test utilities
    mod.rs
    mock_memory.rs            # Mock memory backend for tests
```

## Key Types

- `MemoryRouter` — Routes memory queries to the appropriate backend based on type
- `EpisodicMemory` — Time-ordered experience storage
- `SemanticMemory` — Knowledge and concept storage
- `ProceduralMemory` — Learned patterns and skills
- `SelfMemory` — Self-model and introspective memory

## Architecture Note

This crate provides the **backend storage** layer. The **executive-level memory** (CoreMemory, RecallMemory, ArchivalMemory, compressor, pipeline, scope) lives in `executive/src/impl/memory/`. The ABI trait definitions (`MemoryBackend`, `MemoryEntry`, etc.) live in `fabric/src/memory.rs`.

## Related Docs

- [memory/memory-system.md](memory-system.md) — Full memory system design (migrated from core/memory-system.md)
