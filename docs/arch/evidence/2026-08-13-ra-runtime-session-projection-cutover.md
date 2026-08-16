# RA Runtime session projection/normalization cutover — 2026-08-13

Pure canonical session-item normalization and model-message projection now live
in Runtime:

- `runtime::compaction_normalize`
- `runtime::session_projection`

This includes tool call/result pairing, orphan-result conversion, compaction
lineage, safe tail cuts, ordered canonical message projection and bounded tool
results. Aletheon production callers use Runtime directly; Executive retains
compatibility re-exports for its remaining session service/tests.

Validation:

- `bash scripts/cargo-agent.sh test -p runtime --lib`: 144 passed.
- Executive and Aletheon `--all-targets`: passed.
