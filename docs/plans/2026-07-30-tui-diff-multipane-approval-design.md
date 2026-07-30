# TUI Codex Parity: Diff Rendering, Multi-Pane Layout, Granular Approval (D)

**Date:** 2026-07-30
**Status:** Design proposed (decisions open); implementation not started
**Scope:** Bring the `interact` TUI to codex parity on three fronts — (1) a
syntax-highlighted full unified diff (today: a tree-format file/hunk summary),
(2) a multi-pane layout with a dedicated detail pane (today: a single vertical
stack), and (3) granular approval scope selection (today: only y/a/n). This is
Workstream D of the codex-parity wave.

> Roadmap context: D is a pure `crates/interact` (+ a small `crates/fabric`
> protocol addition) workstream. It is **parallelizable with B** (multi-agent
> planning, which lives in `crates/executive`) — the two touch disjoint crates.
> The reference implementation is codex's `codex-rs/tui` (diff renderer:
> `codex-rs/tui/src/diff_render.rs`, 2559 lines; pane split + approval overlay:
> `codex-rs/tui/src/bottom_pane/`). We emulate its rendering strategy but reuse
> aletheon's existing primitives (`syntect` is already a dependency and already
> drives `crates/interact/src/tui/markdown.rs`; the pager overlay at
> `crates/interact/src/tui/pager.rs` is the established full-frame overlay
> pattern).

## 1. Background & Problem

### 1.1 Current TUI capabilities (verified)

**Render / event loop.** The TUI runs a redraw-on-change loop:
`crates/interact/src/tui/app/lifecycle.rs:88` (`while app.running`), drawing via
`draw_with_recorder` (`lifecycle.rs:104-110`) and polling crossterm events with a
50/200ms timeout (`lifecycle.rs:131-139`). Redraws are gated by `needs_redraw`
(`lifecycle.rs:65`, `:104-110`) — the loop is frame-cheap and only rebuilds on
state change.

**Layout.** `crates/interact/src/tui/render/draw.rs:44-64` builds one fixed
vertical stack via `LayoutHelper`: `header (1 row)` → `chat (flex)` →
`input (2 rows)` → `status (1 row)`. Overlays are drawn *after* the stack: the
completion popup (`draw.rs:67-73`), the approval modal (`draw.rs:76-78`), and —
when active — a full-frame pager that *replaces* the stack entirely
(`draw.rs:37-42`). There is no middle/detail pane; anything richer than the chat
transcript must either overwrite the whole frame (pager) or float as a centered
modal (approval).

**Patch rendering (gap 1 target).** When a tool result carries a
`fabric::PatchDelta` (`crates/interact/src/tui/chat.rs:150`, set via
`set_delta`/`finish_with_delta` at `chat.rs:177-186`, `:511-517`), the chat
widget renders a **tree-format summary only** (`chat.rs:252-297`): a
"Files changed:" header, then per file `~ path (N hunks, X → Y bytes)`
(`chat.rs:266-269`) with a `+`/`-`/`~` icon by `change_type` (`chat.rs:261-265`),
capped at 20 files (`chat.rs:260`, `:275-283`), plus a failed-files list
(`chat.rs:285-296`). **No line-level content, no syntax color, no line numbers.**

**Approval dialog (gap 3 target).** `crates/interact/src/tui/approval_dialog.rs`
is a centered modal (`:64-140`) with a `DialogDecision` enum of exactly three
variants — `Approve`, `ApproveForSession`, `Deny` (`:11-15`) — bound to keys
`y` / `a` / `n|d` (`key_to_decision`, `:54-61`). The detail pane shows at most
**14 lines**, unstyled dark-gray, no scroll (`:73` `detail_h … .min(14)`,
`:129-139` `.take(14)`). The decision maps to `TransientApprovalDecision`
(`crates/interact/src/tui/app/key_handler.rs`, DialogDecision→Transient) and is
sent as `ClientRpcRequest::approval_response`
(`crates/fabric/src/protocol/client.rs:476-482`).

**Streaming.** `crates/interact/src/tui/streaming.rs` `StreamController` keeps a
two-region model — committed (stable scrollback) + tail (mutable, currently
streaming) — explicitly "Inspired by Codex's StreamController" (`streaming.rs:1-6`).
It owns the flex chat region; any new pane must not steal rows from it while a
turn streams.

### 1.2 The daemon boundary — what the TUI actually receives

The TUI talks to the daemon over newline-delimited JSON with a **1 MB frame
cap**: `crates/interact/src/acp/transport.rs:8`
(`DEFAULT_MAX_FRAME_BYTES = 1024 * 1024`), enforced on read (`:53`) and write
(`:71-76`). Anything the diff renderer displays must arrive within that frame or
be fetched out-of-band.

The neutral wire DTO for a patch is `fabric::PatchDelta`
(`crates/fabric/src/types/tool.rs:117-147`), carried on `ToolResultMeta.patch_delta`
(`tool.rs:155-156`). It contains **only summaries**: `applied`, `failed`, and
`files_changed` (each `{ path, change_type, hunks_applied, bytes_before,
bytes_after }`, `tool.rs:141-147`). **There is no diff/hunk text on the wire.**

The full text *does* exist upstream, and is discarded at the DTO boundary:

- `platform::structured_patch::PatchHunk.content` holds the combined hunk text
  (lines prefixed `' '` / `'-'` / `'+'`) —
  `crates/platform/src/structured_patch.rs:35-42`.
- The producer `patch_delta()` in
  `crates/corpus/src/tools/tools/apply_patch.rs:283-327` maps
  `StructuredPatchResult` → `fabric::PatchDelta` and copies only the numeric
  summary fields (`apply_patch.rs:306-320`) — the hunk content is dropped.
- The `dry_run` preview path already renders a real unified diff via
  `hunks_diff(hunks)` (`apply_patch.rs:677`, `:727`; `preview_result` at `:535`),
  proving the diff text is available in-process — it is simply never propagated
  to the runtime tool result.

**Consequence:** the TUI cannot render a real unified diff from what it receives
today. A protocol addition is required (§3.1.4).

**Second diff source — approval artifacts.** For an `ApplyCode` approval the full
diff is *already persisted durably* as an artifact:
`ApprovalSnapshot.artifacts` (`crates/fabric/src/types/approval.rs:200`) holds
`ApprovalArtifactRef { kind: "diff", relative_path, sha256 }`
(`approval.rs:110-115`; test fixture writes `coding-diffs/job.diff`,
`approval.rs:368-372`). The approval subject also carries scope axes:
`ApprovalSubject.allowed_scope: Vec<PathBuf>` and `apply_target: Option<PathBuf>`
(`approval.rs:88-89`), and `ApprovalCategory` has 9 variants — `ApplyCode`,
`ActivateGoal`, `SendMail`, `DeleteFile`, `ModifyCalendar`, `GitPush`,
`CapabilityExpansion`, `DaseinModification`, `BudgetExpansion`
(`approval.rs:33-43`) — plus `ApprovalRisk` (`approval.rs:47-52`). These are the
natural granularity axes for gap 3.

### 1.3 The three concrete gaps

1. **No real diff.** `chat.rs:252-297` shows counts, not content. No highlight,
   no line numbers, no ± coloring, no per-line view. Codex renders each hunk with
   syntect highlighting, gutter line numbers, and GitHub-matched add/del tints
   (`codex-rs/tui/src/diff_render.rs`).
2. **Single-pane layout.** `draw.rs:44-64` is one vertical stack; the only ways
   to show detail are a whole-frame pager (`pager.rs`) or a centered modal. There
   is no persistent detail pane beside the transcript.
3. **Coarse approval.** Three global decisions (`approval_dialog.rs:11-15`),
   14-line unstyled detail, no scroll, no way to say "always allow *this tool*"
   or "always allow writes *under this path*". The durable model already supports
   category + path scope (`approval.rs:82-90`) but the wire decision enum
   (`client.rs:199-203`) collapses everything to Approve/ApproveForSession/Deny.

## 2. Goals / Non-goals

**Goals**

1. Render a syntax-highlighted unified diff (per-file, per-hunk, gutter line
   numbers, ± tints, dark/light-aware) in the TUI, reusing the existing `syntect`
   integration (`markdown.rs`).
2. Introduce a **middle detail pane** that hosts the diff (and, later, other
   inspectors) without breaking streaming or the existing overlays.
3. Extend approval to **scoped grants** — at minimum per-tool and per-path — with
   a keyboard-driven selector, mapped onto the already-durable `ApprovalCategory`
   + `allowed_scope` model.
4. Carry the diff text to the TUI within the 1 MB frame budget, with an
   out-of-band fetch fallback for oversized diffs.
5. Add snapshot coverage for the new renderers (aletheon has ~40 TUI files and a
   single snapshot harness today; codex has 605 `.snap` files).

**Non-goals**

- Mouse-driven diff selection or inline diff editing.
- A full theme-picker / live theme switching (codex has one; we hardcode a
  dark + light pair keyed off terminal background, §3.1.3).
- Word-level intra-line diff highlighting (codex does line-level; we match that).
- Re-architecting `StreamController`; the middle pane borrows from the chat
  flex region, it does not change the streaming model.
- Changing the durable approval contract semantics in `approval.rs` (one-time
  resolution, expiry, subject hashing stay exactly as-is).
- Per-agent MCP scoping and any change to `crates/executive` (that is B's crate).

## 3. Design

### 3.1 Feature 1 — Syntax-highlighted unified diff

#### 3.1.1 Where the diff text comes from (protocol)

**Option A — inline the rendered diff in `PatchDelta`.** Add
`unified_diff: Option<String>` to `fabric::PatchDelta` (`tool.rs:117-121`) and
populate it in `patch_delta()` (`apply_patch.rs:283`) by reusing `hunks_diff`
(`apply_patch.rs:727`). Simple; one hop; already-proven producer. Risk: a large
multi-file diff can blow the 1 MB frame (§4).

**Option B — inline per-file structured hunks.** Add
`files_changed[i].hunks: Vec<{old_start,new_start,content}>` mirroring
`PatchHunk` (`structured_patch.rs:35-42`). Lets the TUI compute line numbers
precisely and re-highlight per file. More faithful, but duplicates the
structured-patch schema into the neutral DTO and is heavier on the wire.

**Option C — reference + fetch.** Send only a diff `sha256` + a size, and add a
`ClientRpcRequest::ArtifactGet { sha256 }` that streams the diff out-of-band
(chunked, bypassing the 1 MB single-frame cap). This is the *only* option that
works for the approval path, where the diff is already a durable artifact
(`approval.rs:110-115`) and may be arbitrarily large.

**RECOMMENDED: A for the inline tool-result path, C for approvals, sharing one
renderer.**
- Tool results: add `unified_diff: Option<String>` to `PatchDelta` (Option A),
  populated from `hunks_diff`, **but bounded** — the producer truncates to a
  configurable byte budget (default ~256 KB, well under the 1 MB frame) and sets
  a `diff_truncated: bool`. Small/normal diffs (the vast majority) render fully
  inline with zero extra round-trips.
- Approvals: the diff is already persisted as an artifact
  (`ApprovalArtifactRef`, `approval.rs:110-115`); add a lightweight
  `ClientRpcRequest::ApprovalArtifactGet { approval_id, kind }` (Option C) that
  reads `coding-diffs/*.diff` and returns it chunked. The TUI lazily fetches when
  the user opens the diff pane — so the approval notification frame stays tiny.
- Both feed the same `DiffView` renderer (§3.1.2). Rejected: B alone (schema
  duplication); C alone for tool results (a round-trip on every small patch).

Trade-off accepted: two ingestion paths, one renderer. The producer already has
`hunks_diff`, so A is nearly free; C is required regardless for oversized/approval
diffs, so we build it once.

#### 3.1.2 The renderer — `DiffView`

New module `crates/interact/src/tui/diff_view.rs` exposing:

```rust
pub struct DiffView {
    files: Vec<DiffFile>,     // parsed from unified-diff text
    scroll: usize,
    theme: DiffTheme,         // Dark | Light (resolved once, §3.1.3)
}
pub struct DiffFile { path: String, change: ChangeKind, hunks: Vec<DiffHunk> }
pub struct DiffHunk { old_start: u64, new_start: u64, lines: Vec<DiffLine> }
pub enum DiffLine { Context(String), Add(String), Del(String) }
```

- **Parse.** Reuse `platform::structured_patch::parse_unified_diff`
  (`structured_patch.rs:451`) to turn the unified-diff string into
  `PatchOperation`/`PatchHunk`, then flatten into `DiffLine`s. This avoids a
  second diff parser and keeps the TUI honest against the same grammar the
  applier uses.
- **Line numbers.** Track `old`/`new` counters from each hunk header
  (`old_start`/`new_start`, `structured_patch.rs:36-39`): context advances both,
  `Del` advances old, `Add` advances new. Render a two-column gutter
  `old │ new │ <sign> code`.
- **Syntax highlight.** Reuse the existing syntect setup already proven in
  `crates/interact/src/tui/markdown.rs` (`SyntaxSet::load_defaults_newlines()`,
  `ThemeSet`, `HighlightLines`, `base16-ocean.dark`). Pick the syntect syntax by
  file extension from `DiffFile.path`; highlight the *code* portion of each line
  (after the ±/space sign), then overlay the add/del background tint. Preserve
  syntect parser state across consecutive lines within a hunk (codex does this at
  `diff_render.rs` — "preserves syntect's parser state across consecutive lines
  within a hunk"); reset at hunk boundaries.
- **Colors.** Add = green fg + subtle green bg; Del = red fg + subtle red bg;
  gutter = dark-gray; context = default. Match codex's palette intent
  (`diff_render.rs:58-60`: "Light-theme values match GitHub's diff colors").

#### 3.1.3 Themes

**RECOMMENDED:** ship a `DiffTheme::{Dark, Light}` pair, resolved once per open
from the terminal background (crossterm can query; fall back to Dark). Codex
resolves a `DiffTheme` per render and probes the active syntax theme
(`diff_render.rs:118`, `:191-227`); we take the simpler fixed-pair route (a full
theme picker is a non-goal). The syntect code theme stays `base16-ocean.dark` for
Dark and a light base16 for Light, matching `markdown.rs`.

#### 3.1.4 Wire changes summary

- `fabric::PatchDelta` (`tool.rs:117-121`): `+ unified_diff: Option<String>`,
  `+ diff_truncated: bool` (both `#[serde(default)]` for back-compat).
- `apply_patch.rs:283-327` `patch_delta()`: populate `unified_diff` from
  `hunks_diff` with a byte budget.
- `ClientRpcRequest` (`client.rs:55`): `+ ApprovalArtifactGet { approval_id,
  kind }` and a chunked response, for the approval diff path only.

### 3.2 Feature 2 — Multi-pane layout

#### 3.2.1 Options

**Option A — split the chat flex region horizontally when detail is active.**
When a detail (diff) is open, split the flex row into `chat (left, ~55%)` +
`detail (right, ~45%)`; when closed, chat reclaims the full width. Header, input,
status stay full-width top/bottom. Streaming keeps writing to the (narrower)
chat pane; `StreamController` is unaffected because it owns *content*, not
geometry.

**Option B — replace the pager overlay with a detail overlay.** Reuse the
`pager.rs` full-frame overlay pattern (`draw.rs:37-42`) but render the diff
instead of the transcript. Zero layout risk (it is the proven overlay path), but
it *hides* the conversation — no side-by-side, no parity with codex's persistent
panes.

**Option C — three-region vertical stack (codex-style top/middle/bottom).**
Codex splits `top_pane / middle_pane / bottom_pane` (`bottom_pane/`); mirror that
with `chat (top flex) / detail (middle, fixed height) / input+status (bottom)`.
Simple to reason about but squeezes the transcript vertically and fights the
streaming tail for rows.

**RECOMMENDED: A (horizontal split of the flex region), with B's overlay kept as
the "maximize" affordance.** Rationale: A preserves streaming (the chat pane
keeps its own scrollback/tail; only its width changes), gives true side-by-side
parity with codex, and is a *localized* change to `draw.rs:44-64` — introduce the
split only inside the existing flex slot. The pager overlay (`pager.rs`) is
retained and repurposed as "press `f` to full-screen the diff", giving the
best of both. Rejected C: it starves the streaming region of vertical space,
which the two-region `StreamController` (`streaming.rs`) needs most.

#### 3.2.2 Implementation shape

- Add `app.detail: Option<DetailPane>` where `DetailPane::Diff(DiffView)`
  (state in `crates/interact/src/tui/mod.rs` alongside `pending_approval`,
  `mod.rs:324`).
- In `draw.rs`, replace the single `push_flex(ChatRenderable…)` (`draw.rs:48-52`)
  with: if `app.detail.is_some()`, wrap the flex slot in a
  `Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])`
  and render `ChatRenderable` left, `DiffView` right; else render chat full-width
  as today. A minimum-width guard collapses to chat-only + overlay under a
  threshold (§4).
- Keyboard (in `app/key_handler.rs`): `Ctrl+D` toggles the diff pane for the last
  patch-bearing tool result; `f` promotes it to the full-frame pager-style
  overlay; `j/k`/`PgUp`/`PgDn` scroll it when focused (mirrors `pager.rs:123-160`).
- Streaming safety: the detail pane is rendered only when explicitly opened and
  never during the auto-open critical path; `needs_redraw` (`lifecycle.rs:65`)
  already coalesces the extra render. No change to `StreamController`.

#### 3.2.3 Target layout (ASCII mock)

```
┌───────────────────────────────────────────────────────────────────────┐
│ aletheon · session 3f2a · model opus-4.8 · budget 42%          [header]│
├──────────────────────────────────┬────────────────────────────────────┤
│ chat transcript (flex, left ~55%)│ Diff — src/tui/diff_view.rs   [det.]│
│                                  │ 12 │ 12 │  use ratatui::layout::… │
│ ⏺ apply_patch(diff) — done       │ 13 │ 13 │  use syntect::easy::…   │
│   ├─ Files changed: 2            │    │ 14 │+ pub struct DiffView {  │  ← add (green)
│   │  ~ diff_view.rs (3 hunks)    │ 14 │    │- struct OldView {       │  ← del (red)
│   ▏streaming tail…               │ 15 │ 15 │      scroll: usize,     │
│                                  │ ───┼────┼── @@ -40,6 +41,9 @@ ──── │  ← hunk sep
│                                  │ 40 │ 41 │      files,             │
├──────────────────────────────────┴────────────────────────────────────┤
│ > _                                                             [input] │
│ Ctrl+D diff · f full · a approve-scope · ?help              [status]    │
└───────────────────────────────────────────────────────────────────────┘
Narrow terminal (< ~100 cols): detail pane collapses; `f` opens full-frame
overlay instead (reuses pager.rs render path).
```

### 3.3 Feature 3 — Granular approval scope

#### 3.3.1 Scope model

The durable layer already encodes the axes we need — do **not** invent a new one:
- **per-tool / per-category:** `ApprovalCategory` (9 variants,
  `approval.rs:33-43`).
- **per-path:** `ApprovalSubject.allowed_scope: Vec<PathBuf>` +
  `apply_target` (`approval.rs:88-89`).
- **per-command:** for `GitPush` / exec-style categories, the command string
  lives in `ApprovalSubject.attributes` (`approval.rs:87`).

The gap is purely on the *wire decision*: `TransientApprovalDecision`
(`client.rs:199-203`) has only `Approve | ApproveForSession | Deny`.

**RECOMMENDED scope enum** — extend the decision, keeping the three existing
variants for back-compat:

```rust
pub enum TransientApprovalDecision {
    Approve,             // once (unchanged)
    ApproveForSession,   // any operation this session (unchanged, = current 'a')
    Deny,                // (unchanged)
    ApproveToolForSession,   // NEW: any op of this ApprovalCategory this session
    ApprovePathForSession,   // NEW: any op whose apply_target is under this root
}
```

`ApprovalResponseParams` (`client.rs:216-218`) gains an optional
`scope_hint: Option<{ category?, path? }>` so the daemon can persist the grant
against the right axis. The daemon side (executive) is **out of D's scope** (it
is B's crate) — D defines the wire contract and the TUI UX; a follow-up wires the
executive-side session grant table. Until then the two new variants degrade
safely to `ApproveForSession` on the daemon (documented open decision).

Granularity chosen: **per-tool (category) and per-path**. Per-command is deferred
(it needs command normalization the durable layer only stores as opaque
attributes today).

#### 3.3.2 Keyboard UX + dialog upgrade

Replace the flat 3-key modal (`approval_dialog.rs:54-61`) with a
selector-augmented modal, borrowing codex's `list_selection_view` +
`approval_overlay` pattern (`codex-rs/tui/src/bottom_pane/approval_overlay.rs`,
which distinguishes `CommandExecutionApprovalDecision` /
`FileChangeApprovalDecision` and a `GrantForSession` scope):

- `y` — approve once (unchanged).
- `a` — approve for session (unchanged).
- `t` — approve this **tool/category** for the session (`ApproveToolForSession`).
- `p` — approve writes under this **path** for the session
  (`ApprovePathForSession`, shown only when `apply_target`/`allowed_scope` is
  present).
- `n` / `d` / `Esc` — deny (unchanged).
- The hint row (`approval_dialog.rs:141-162`) lists only the scopes valid for the
  request's `ApprovalCategory` (e.g. `p` hidden for `SendMail`).

`key_to_decision` (`approval_dialog.rs:54-61`) and the DialogDecision→Transient
mapping in `app/key_handler.rs` extend accordingly; `DialogDecision`
(`approval_dialog.rs:11-15`) gains the two new variants.

#### 3.3.3 Scrollable, highlighted approval detail

Today the detail pane is 14 lines, unstyled, unscrollable
(`approval_dialog.rs:73`, `:129-139`). Upgrade it to host a `DiffView` (§3.1.2)
when the approval carries a diff artifact (`ApprovalCategory::ApplyCode`,
artifact `kind == "diff"`, `approval.rs:110-115`), fetched lazily via
`ApprovalArtifactGet` (§3.1.1 Option C). Non-diff approvals keep a scrollable
plain-text detail (add `scroll` + `j/k` to the modal). This is where features 1
and 3 converge on one renderer.

## 4. Error handling

- **Oversized diffs (tool path):** producer truncates `unified_diff` to the byte
  budget and sets `diff_truncated` (§3.1.4); the TUI renders what fits and shows
  `… diff truncated — press f for full view` which triggers the chunked fetch
  (§3.1.1 C). Guarantees the 1 MB frame cap (`transport.rs:8`, `:53`) is never
  exceeded.
- **Oversized diffs (approval path):** never inlined — always fetched chunked via
  `ApprovalArtifactGet`, so frame size is independent of diff size.
- **Binary files:** `parse_unified_diff` (`structured_patch.rs:451`) yields no
  `+/-` text lines for a binary hunk; `DiffView` detects an empty/`Binary files
  differ` hunk and renders `⯄ binary file (N bytes → M bytes)` from the existing
  `bytes_before`/`bytes_after` summary (`tool.rs:145-146`) — no highlight
  attempted.
- **Missing highlight grammar:** if syntect has no syntax for the extension, fall
  back to plain (uncolored) diff lines with gutter + ± tint intact — exactly how
  `markdown.rs` degrades for unknown fences. Never fail the render.
- **Narrow terminals:** the horizontal split (§3.2.2) requires a min width
  (~100 cols); below it, the detail pane is suppressed and `f` opens the
  full-frame overlay instead (reusing `pager.rs`), which already handles any
  width. The modal (§3.3.3) keeps its existing `popup_w` clamp
  (`approval_dialog.rs:72`).
- **Malformed diff text:** if `parse_unified_diff` errors, `DiffView` falls back
  to rendering the raw diff string as plain monospace (no crash), preserving the
  current worst-case behavior.
- **Missing artifact / fetch failure:** `ApprovalArtifactGet` returning an error
  leaves the modal on the plain-text summary and shows `diff unavailable`; the
  approval decision remains fully usable (fail-open on *display*, never on the
  decision).

## 5. Verification

**Snapshot tests (extend the existing harness).** aletheon has one TUI snapshot
harness (`crates/interact/tests/tui_snapshots.rs`, using
`interact::tui::reducer::{reduce, snapshot_view}` and `AppState`) and a
`crates/interact/tests/snapshots/` dir; codex has 605 `.snap` files. Add:
- `diff_view.rs` unit + snapshot: add/del/context coloring, gutter line numbers,
  multi-hunk separators, unknown-grammar fallback, binary-file case, truncation
  banner. (New module gets its own `#[cfg(test)]` + a `tui_snapshots.rs` case.)
- Layout snapshot: chat-only vs chat+detail split at wide width; collapse to
  chat-only at narrow width.
- Approval modal snapshot: scope-hint hint row per `ApprovalCategory` (e.g.
  `ApplyCode` shows `t`/`p`; `SendMail` shows only `t`); scrollable diff detail.

**tmux / scenario tests.** Drive real key sequences through the existing
scaffolding:
- `tests/tui_tmux` — a scenario that submits a task producing an `apply_patch`
  result, presses `Ctrl+D` to open the diff pane, `f` to maximize, `j/k` to
  scroll, `Esc` to close.
- `tests/tui_scenarios` — an approval scenario asserting `t` sends
  `ApproveToolForSession` and `p` sends `ApprovePathForSession` (verify the
  emitted `approval_response` payload).

**Protocol tests.** `fabric` round-trip test for the new `PatchDelta` fields
(default-absent for old peers) and the new `TransientApprovalDecision` variants;
a `transport.rs`-level test that a max-budget `unified_diff` stays under
`DEFAULT_MAX_FRAME_BYTES`.

**Commands (via the wrapper, narrowest first):**

```
bash scripts/cargo-agent.sh test -p interact
bash scripts/cargo-agent.sh test -p fabric
bash scripts/cargo-agent.sh fmt --all -- --check
```

## 6. Files touched

New:
- `crates/interact/src/tui/diff_view.rs` — `DiffView` renderer (parse via
  `structured_patch::parse_unified_diff`, syntect highlight reusing `markdown.rs`
  setup, gutter line numbers, dark/light themes).

Changed (interact):
- `crates/interact/src/tui/render/draw.rs:44-64` — horizontal split of the flex
  slot when `app.detail` is set; narrow-width guard.
- `crates/interact/src/tui/mod.rs:324,388` — add `detail: Option<DetailPane>`
  state beside `pending_approval`.
- `crates/interact/src/tui/app/key_handler.rs` — `Ctrl+D`/`f`/scroll keys;
  extend DialogDecision→`TransientApprovalDecision` mapping.
- `crates/interact/src/tui/approval_dialog.rs:11-15,54-61,129-162` — new scope
  variants, scope-aware hint row, scrollable/highlighted detail hosting `DiffView`.
- `crates/interact/src/tui/chat.rs:252-297` — keep the summary; add an
  "open diff (Ctrl+D)" affordance when `patch_delta.unified_diff` is present.
- `crates/interact/src/tui/response.rs:317-353` — thread the diff artifact ref
  into `pending_approval`; handle `ApprovalArtifactGet` responses.
- `crates/interact/src/tui/mod.rs` (module list) — register `diff_view`.

Changed (fabric — the only cross-crate touch):
- `crates/fabric/src/types/tool.rs:117-121` — `unified_diff: Option<String>` +
  `diff_truncated: bool` on `PatchDelta`.
- `crates/fabric/src/protocol/client.rs:199-203,216-218` — two new
  `TransientApprovalDecision` variants + optional `scope_hint`; new
  `ApprovalArtifactGet` request/response.

Changed (corpus — producer):
- `crates/corpus/src/tools/tools/apply_patch.rs:283-327` — populate
  `unified_diff` via `hunks_diff` (`:727`) under a byte budget.

Explicitly **unchanged:** `crates/interact/src/acp/transport.rs` (the framing
contract is honored, not modified); `crates/platform/src/structured_patch.rs`
(reused as-is); `crates/fabric/src/types/approval.rs` (durable contract unchanged).

## 7. Scope boundary

D lives in `crates/interact` plus a small, additive `crates/fabric` protocol
change and one `crates/corpus` producer line. It is **parallelizable with B**
(multi-agent planning), which lives entirely in `crates/executive` — no shared
files. The executive-side *persistence* of scoped session grants (honoring the
two new `TransientApprovalDecision` variants) is deliberately **excluded from D**:
D ships the wire contract + TUI UX, and the new variants degrade to
`ApproveForSession` on the daemon until B (or a dedicated follow-up) wires the
grant table. D also excludes: theme picker, mouse selection, word-level intra-line
diff, per-command scope, and any change to `StreamController`.

## Open decisions for review

1. **Diff transport split (§3.1.1):** approve "inline-bounded for tool results
   (Option A) + chunked fetch for approvals (Option C)"? Or force a single path
   (all-fetch) for uniformity at the cost of a round-trip on every small patch?
2. **Inline diff byte budget:** is ~256 KB the right cap for `unified_diff`
   before truncation, given the 1 MB frame (`transport.rs:8`) shares space with
   other result fields?
3. **Layout (§3.2.1):** horizontal split (A) as primary with the overlay (B) as
   "maximize" — agreed? Or is the codex-style vertical top/middle/bottom (C)
   preferred despite squeezing the streaming region?
4. **Narrow-terminal threshold:** is ~100 cols the right cutoff for collapsing the
   split to overlay-only?
5. **Approval scope granularity (§3.3.1):** ship per-tool + per-path now, defer
   per-command? And is degrading the two new decision variants to
   `ApproveForSession` on the (unmodified) daemon an acceptable interim, or must
   the executive grant table land in the same wave?
6. **Scope keys:** `t` (tool) / `p` (path) — acceptable, or prefer a codex-style
   inline list-selector (arrow keys + Enter) over single-key scopes?
7. **Themes (§3.1.3):** fixed Dark/Light pair keyed off terminal background —
   sufficient, or is a live theme picker in-scope for parity?
