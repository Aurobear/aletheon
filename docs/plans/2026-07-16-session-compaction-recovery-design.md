# Session Compaction and Recovery Repair Design

## Problem

The deployed daemon can leave a TUI turn permanently waiting when context
compaction truncates a multibyte tool argument at an invalid UTF-8 byte
boundary. The same incident also showed that oversized tool results rapidly
refill the active context and that sandbox write restrictions can be
misreported as a read-only host filesystem.

Runtime evidence is the session
`e8777ced-5989-44c6-87d1-4de973d76462`: three worker panics point to
`crates/fabric/src/include/compaction.rs:119`, and the two user messages after
the first panic have no assistant completion.

## Goals

- Make every string bound UTF-8 safe.
- Keep oversized tool output durable while bounding its model-visible form.
- Preserve the newest user request and causal tool boundary during compaction.
- Convert compaction or turn-task failure into a terminal, user-visible result.
- Describe sandbox mutation denial accurately and never recommend remounting
  the host merely because a sandbox path is read-only.
- Deploy the exact tested source HEAD and verify it through the real TUI path.

## Non-goals

- Expanding writable workspace scope automatically.
- Changing provider credentials or production token limits.
- Deleting or rewriting the affected session journal.
- Treating a successful RPC-only request as TUI acceptance evidence.

## Design

### UTF-8-safe bounds

Introduce one small byte-budget helper that walks backward from the requested
limit to a valid character boundary. Use it in compaction tool-argument
pruning and other adjacent error/tool string bounds reached by the same turn
path. Tests cover Chinese, emoji, ASCII, and strings already under budget.

### Context shaping

Before an LLM continuation, replace oversized tool results in the active
message buffer with bounded summaries while leaving the event journal
unchanged. Compaction continues to retain a tool-boundary-safe recent tail and
an older summary. The latest user message must remain verbatim after a normal
or forced compaction. A second compaction over multibyte tool inputs must be
deterministic and panic-free.

### Failure completion

Compaction remains fallible. Its callers record the error and continue only
when the remaining context is within the provider window; otherwise the turn
finishes with an explicit error. Spawned turn-task join failures are mapped to
the same terminal protocol event so the TUI restores its input prompt rather
than waiting indefinitely.

### Sandbox diagnostics

Mutation-boundary errors state that the denial comes from the configured
sandbox/working-directory policy, not the host mount. The diagnostic offers
safe actions: relaunch from the intended working directory or choose a path
inside the approved workspace. It explicitly rejects mount/remount advice.

## Verification

1. Unit tests for multibyte truncation and second compaction.
2. Session-manager recovery tests preserving the latest user request.
3. Turn-handler tests proving compaction/join failure emits a terminal event.
4. Sandbox diagnostic tests forbidding remount guidance.
5. Strict Clippy and relevant crate/workspace tests.
6. Build the release binary, verify its SHA-256, install it, and restart the
   `aletheon` systemd unit.
7. Run the same real-TUI workflow three times from the intended canonical
   working directory; retain final frames, session paths, journal errors,
   deployed hash, unit properties, and source commit.

## Rollback

Preserve the previous installed binary before replacement. If startup or any
smoke assertion fails, restore that binary and restart the unit. Source
rollback is limited to the repair commits; unrelated working-tree changes are
never staged or reverted.
