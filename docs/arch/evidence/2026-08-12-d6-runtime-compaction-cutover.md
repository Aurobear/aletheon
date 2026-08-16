# D6 Runtime compaction cutover evidence

- Date: 2026-08-12
- Canonical owner: `runtime::compaction`.
- The compactor port, strategy/outcome/failure contracts, pruning helpers, UTF-8 truncation, degenerate-summary detection, and tool-pair-safe tail selection moved together to Runtime.
- Cognit, Corpus, Mnemosyne, Executive and Runtime consumers use the Runtime owner directly.
- Fabric's module, root re-exports, implementation/tests, and four census/public rows were deleted with no compatibility re-export.
