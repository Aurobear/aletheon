# TUI Execution Visibility and Information Hierarchy Design

## Problem statement

The latest installed-runtime session (`8aa63749-d8ac-454c-971d-a3bf47da400f`)
persisted tool calls at session sequences 13-14 and 22-35, but the normal TUI
made their arguments, results, and failures difficult to inspect. Several
read-only probes were also rejected as mutations, while the final response
presented conclusions more confidently than the runtime evidence justified.

The implementation must improve execution visibility without exposing or
persisting private chain-of-thought. It must distinguish provider inference,
host-authored progress, tool execution, and the final answer.

## Current code anchors

- Tool entries begin collapsed and render output only when expanded:
  `crates/interact/src/tui/chat.rs:158-168`,
  `crates/interact/src/tui/chat.rs:233-259`.
- `Ctrl+B` targets only the most recent tool entry:
  `crates/interact/src/tui/app/key_handler.rs:252-260`.
- Thinking is collapsed at turn start and discarded after a duration marker is
  committed: `crates/interact/src/tui/streaming.rs:87-96`,
  `crates/interact/src/tui/streaming.rs:278-288`.
- The main layout is a one-row header, chat, input, and status line:
  `crates/interact/src/tui/render/draw.rs:52-72`.
- The public conscious trace intentionally excludes hidden reasoning:
  `crates/fabric/src/types/conscious_core_trace.rs:1-5`.

## Goals

1. Make every tool call discoverable in chronological order, including its
   meaningful arguments, terminal status, duration, and bounded output.
2. Show safe, host-authored activity summaries instead of raw hidden reasoning.
3. Correctly classify common read-only diagnostic commands.
4. Separate conversation, activity, detail, and runtime status visually.
5. Preserve authoritative session/tool evidence and existing security
   boundaries.

## Non-goals

- Persisting or displaying raw chain-of-thought.
- Allowing arbitrary shell commands merely because they contain `--help`.
- Replacing the session schema with UI-specific payloads.
- Adding a new top-level crate or a second execution-event authority.

## Proposed experience

```text
┌ Aletheon · active_agent · model · context · cache ┐
│ Conversation                              Detail  │
│ User request                              tool    │
│ Activity 3/5                              args    │
│ ├─ Read gbrain-memory                     output  │
│ ├─ Search 4 paths: no matches             status  │
│ └─ Check service: policy denied                     │
│                                                       │
│ Final answer                                          │
├ Input                                                 │
└ cwd · inference 4 · tools 5/8 · elapsed · context     ┘
```

The layout remains usable on narrow terminals. The detail pane appears only
when there is sufficient width; otherwise it becomes an overlay opened from a
selected activity item.

## Architecture

### 1. Activity presentation model

Extend the existing `ChatEntry::Exec` presentation state rather than inventing
a new runtime event schema. Each entry derives a bounded presentation from the
authoritative `ToolCallStart`, `ToolProgress`, and terminal `ToolCallResult`:

- normalized action label;
- bounded argument summary;
- running/succeeded/failed/denied status;
- elapsed duration;
- bounded output preview;
- reference to the full in-memory output for the detail view.

Selection belongs to `ChatWidget`: previous/next activity navigation, toggle
selected item, and open/close detail. Existing `Ctrl+B` remains as a compatibility
shortcut but operates on the selected item when one exists.

### 2. Safe progress summaries

Provider `ThinkingDelta` remains transient and private. The completed transcript
stores only a neutral duration marker. User-visible progress is instead derived
from typed host events:

```text
Inspecting configured memory capability
Searching four requested paths
Command rejected by read-only policy
Synthesizing from partial evidence
```

These summaries describe observable actions, never model rationale or hidden
tokens. They may be rendered as activity rows but must not be inserted into a
future model context as assistant reasoning.

### 3. Read-only command classification

The command policy must parse the executable and arguments, not use substring
allowlisting. Add narrowly bounded recognition for:

- `<executable> --help`, `<executable> -h`, and `<executable> help` when no
  shell operators or additional mutating subcommands are present;
- `systemctl status`, `show`, `is-active`, `is-enabled`, `list-units`, and
  `list-unit-files` without mutating flags;
- existing read-only filesystem and repository probes.

Pipelines and compound shell expressions retain their current aggregate risk
classification. A read-only prefix must never downgrade a later mutating
segment.

### 4. Evidence-aware completion

The TUI shows a turn summary derived from authoritative events:

```text
4 inference rounds · 8 tool calls · 4 succeeded · 3 denied · 1 empty result
```

Failed or denied calls remain visually prominent after the turn completes.
This does not rewrite model prose. A separate system prompt rule should require
the model to distinguish documented design, observed runtime facts, and
unverified inference when producing the final answer.

### 5. Information hierarchy

- Header: agent/profile, effective model, connection state, active-context
  pressure, and provider cache state when known.
- Conversation: user and final assistant messages remain primary.
- Activity: compact chronological rows between request and answer.
- Detail: selected tool arguments, output, error, patch information, and IDs.
- Footer: canonical cwd, inference rounds, retries, tool success/total, elapsed
  time, and active-context occupancy. Cumulative billed tokens remain separate.

Unknown runtime values render as `—`, not fabricated zeroes.

## Error handling

- Missing terminal tool results render as `incomplete`, never `succeeded`.
- Policy denials use a distinct state from execution failures.
- Output truncation reports original line/byte counts and preserves artifact
  references when available.
- When detail data is unavailable after session resume, the row remains visible
  and identifies that only the persisted bounded record is available.
- Provider cache telemetry remains `unknown` when the provider did not report
  it.

## Validation

### Deterministic tests

- Tool activity selection, navigation, expansion, denial styling, duration,
  and narrow-terminal fallback.
- Snapshot tests for conversation/activity/detail hierarchy.
- Command-policy tests proving the allowed read-only forms and rejecting
  compound or mutating variants.
- Tests proving hidden thinking is neither persisted nor included in model
  context.
- Turn summary tests keeping inference rounds, retries, tool calls, cache usage,
  cumulative usage, and active-context occupancy separate.

### Installed-runtime acceptance

After `sudo bash scripts/aletheon.sh deploy`, use `/usr/bin/aletheon` and the
official user socket for a real multi-tool task. Acceptance requires:

1. every tool call is visible and selectable;
2. arguments and bounded output are inspectable;
3. a read-only `--help` and `systemctl show/status` probe succeeds;
4. a mutating systemctl command remains denied or requires authorization;
5. no raw hidden reasoning is persisted;
6. the final frame separates activity from the final answer;
7. daemon restart counters remain stable and the installed binary digests
   match the release artifact.

## Scope and delivery order

1. Correct read-only command classification and contract tests.
2. Add the activity selection/detail presentation model.
3. Add typed activity summaries and per-turn evidence counters.
4. Rework header/footer and responsive detail layout.
5. Validate through deterministic snapshots and installed real-TUI runs.
