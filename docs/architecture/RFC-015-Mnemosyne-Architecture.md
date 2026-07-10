# RFC-015 Mnemosyne Architecture

## Purpose

Mnemosyne is the experience system of Aletheon.

It manages knowledge across time.

## Memory Types

-   Episodic
-   Semantic
-   Procedural
-   Self

## Background Services

-   Replay
-   Consolidation
-   Association
-   Compression
-   Importance Update
-   Forgetting
-   Embedding

## Relationship

Agora -\> Commit -\> Mnemosyne

Mnemosyne -\> Recall -\> Agora

## Suggested Modules

src/ ├── recall/ ├── storage/ ├── association/ ├── replay/ ├──
consolidation/ ├── embedding/ ├── index/ ├── background/ └── api/
