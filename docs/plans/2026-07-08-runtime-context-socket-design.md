# Runtime Context Budget And Socket Access Design

## Problem

The daemon currently persists an enriched user message containing transient
skill and memory injections. That enriched message is restored as conversation
history and can be injected again on later turns. In addition, the cache-stable
prefix includes every skill's full body rather than its short description, and
individual recalled facts and activated skills have no prompt-size budget.

The system socket is intentionally mode `0660` and owned by the `aletheon`
group. Adding a user to that group does not update supplementary groups in an
already-running login session, so the kernel can reject the socket open before
the daemon's peer-credential authorization runs.

## Context Assembly

Each LLM request is assembled from four bounded layers:

```text
system prefix (base prompt + skill summaries + core-memory snapshot)
    + recent persisted raw conversation history
    + bounded transient recall/activated-skill/Dasein context
    + current raw user message
```

- Persist only the raw user message in SessionManager, EventJournal, and
  RecallMemory.
- Never persist transient prompt decorations.
- Build the ReAct seed explicitly for each turn and include the system prefix
  exactly once.
- PrefixBuilder renders skill name and description, never all skill bodies.
- Activated skill and recalled fact text are truncated under explicit
  per-item and total character budgets.
- Provider/context-overflow errors are returned to the client but are not
  stored as assistant conversation, RecallMemory, or AutoMemory input.

## Socket Access

- Keep the socket at `0660`; do not restore world-writable permissions.
- After adding the invoking user to the `aletheon` group, the installer checks
  the current process group list.
- If the group is not active, print a prominent instruction to re-login or run
  `newgrp aletheon`, plus the temporary command
  `sg aletheon -c 'aletheon'`.
- Documentation explains that group database membership and active process
  supplementary groups are different states.

## Verification

- A two-turn test proves transient injection appears in the LLM request but
  never in persisted history.
- A recovery test proves restarting from the journal cannot restore injected
  skill or recall content.
- Prefix and recall tests verify configured size bounds.
- Error-path tests prove context overflow responses are not persisted.
- Installer shell tests or deterministic output checks cover the inactive-group
  warning while socket authorization tests continue to require `0660`.

