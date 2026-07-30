# TUI Diff / Detail / Scoped Approval Implementation Plan (D)

**Design:** `docs/plans/2026-07-30-tui-diff-multipane-approval-design.md`

1. Add backward-compatible diff preview/artifact metadata and typed scope subjects/decisions to Fabric.
2. Make every host `RequireApproval` verdict authoritative regardless of static tool level; derive ApplyPatch mutation targets from validated typed input.
3. Persist exact-tool and path-scoped thread grants in SQLite, validate subject version/hash/path before resolving, and consult them at the daemon approval gate after restart. Never convert a path grant into a tool grant.
4. Persist complete content-addressed diff text, inline a UTF-8-safe 64 KiB preview, and page full text in independently capped 64 KiB RPC responses.
5. Add a line-numbered colored DiffView, wide-terminal detail pane, maximize/scroll controls, and scope-aware approval dialog.
6. Validate Fabric/Corpus/Executive/Interact tests, then installed TUI diff paging and approval evidence.
