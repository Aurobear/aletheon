# cognit

BrainCore cognitive engine — the "brain" of the Aletheon agent.

## Overview

The `cognit` crate implements the cognitive architecture that enables agents to think, reason, learn, and reflect. It contains the BrainCore and its six cognitive components.

## Architecture

```
core/
├── planner.rs       — Goal decomposition and action planning
├── reasoner.rs      — Logical inference and chain-of-thought
├── critic.rs        — Quality evaluation and error detection
├── learner.rs       — Knowledge acquisition and adaptation
├── reflector.rs     — Self-reflection and meta-cognition
├── world_model.rs   — Environmental state representation
├── awareness.rs     — Consciousness signals
└── skill_extractor.rs — Skill extraction from experience

impl/
├── llm/             — LLM provider integrations (Anthropic, OpenAI, Ollama)
├── inference/       — Inference pipeline
├── learning/        — Learning mechanisms
└── grounding/       — Context grounding
```

## Key Types

- `BrainCore` — Main cognitive orchestrator
- `Planner` — Goal decomposition
- `Reasoner` — Chain-of-thought reasoning
- `Critic` — Quality evaluation
- `Learner` — Knowledge acquisition
- `Reflector` — Self-reflection
- `WorldModel` — Environmental state

## Usage

```rust
use cognit::{BrainCore, Planner, Reasoner, Critic};
```

## Dependencies

- `base` — Core traits and types
