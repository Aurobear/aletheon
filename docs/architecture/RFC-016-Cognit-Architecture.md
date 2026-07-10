# RFC-016 Cognit Architecture

## Purpose

Cognit is the cognitive core.

## Pipeline

Intent → Context → Agora → Planner → Reasoner → Verifier → Decision →
Corpus

## Components

-   Planner
-   Reasoner
-   World Model
-   Verifier
-   Reflector
-   Learner
-   Attention
-   Harness

Future versions should replace a fixed ReAct loop with configurable
Harness graphs.

## Suggested Layout

src/ ├── planner/ ├── reasoner/ ├── verifier/ ├── reflector/ ├──
learner/ ├── awareness/ ├── harness/ └── api/
