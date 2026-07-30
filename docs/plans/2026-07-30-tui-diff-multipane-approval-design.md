# TUI Codex Parity: Diff Rendering, Multi-Pane Layout, Granular Approval (D)

**Date:** 2026-07-30
**Status:** Revised design; transport, layout, and scoped-approval enforcement
decisions locked; implementation plan pending
**Scope:** Bring the `interact` TUI to codex parity on three fronts — (1) a
syntax-highlighted full unified diff (today: a tree-format file/hunk summary),
(2) a multi-pane layout with a dedicated detail pane (today: a single vertical
stack), and (3) granular approval scope selection (today: only y/a/n). This is
Workstream D of the codex-parity wave.

> Roadmap context: diff rendering is primarily `interact`/`fabric`/`corpus`, but
> artifact fetch and scoped approval enforcement necessarily touch `executive`.
> D is not a disjoint-client-only workstream and must be sequenced with any B
> changes to shared daemon bootstrap/RPC files.
> The reference implementation is codex's `codex-rs/tui` (diff renderer:
> `codex-rs/tui/src/diff_render.rs`; pane split + approval overlay:
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

There is also a baseline wire mismatch that D must repair before adding new
choices. `TransientApprovalDecision` serializes `Approve` and
`ApproveForSession` as `"approve"` / `"approve_for_session"`
(`client.rs:199-213`), while `AdminService::resolve_transient_approval` matches
only `"once"` / `"always"` and maps every other string to deny
(`crates/executive/src/application/admin_service.rs:531-537`). The RPC handler
currently parses the decision as an untyped string
(`rpc_admin.rs:125-139`). Thus the UI exposes y/a/n, but the current typed RPC
values do not implement the advertised y/a semantics end-to-end. The same-wave
fix is typed request deserialization with one canonical enum vocabulary; do not
add more string aliases as the long-term contract.

The intended `a` scope is already **per tool**, not global. The server stores a
`ThreadGrantKey { owner, tool }` (`fabric/src/types/tool.rs:100-104`),
`ScopedApprovalCache` queries that exact key
(`executive/src/application/admin_service.rs:235-272`), and resolution records
`resolved.tool` for the authenticated thread (`admin_service.rs:554-563`). D
therefore keeps `ApproveForSession` as the compatibility name for "this exact
tool in this thread"; adding a second `ApproveToolForSession` variant would be a
duplicate contract.

Finally, `PolicyVerdict::RequireApproval` is currently honored only when the
tool's static level is L2+ (`corpus/src/security/runner.rs:328-330`). D makes the
host policy verdict authoritative at every level, while preserving the existing
L2+ default policy. This is what allows an operator rule such as typed
`ask apply_patch` to produce a real path-scoped approval without reclassifying
the tool (`apply_patch.rs:55-57` currently declares L1) or keying runtime
behavior to prompt text.

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
- `hunks_diff(hunks)` (`apply_patch.rs:727-740`) renders hunk bodies only. It
  does not emit `---`/`+++` file headers, while `parse_unified_diff` requires
  those headers (`structured_patch.rs:451-468`). Also, `patch_delta()` receives
  `StructuredPatchResult`, which has already lost the original hunks
  (`structured_patch.rs:50-59`). A canonical diff must therefore be captured
  from authorized before/after content during apply, not reconstructed from the
  summary DTO.

**Consequence:** the TUI cannot render a real unified diff from what it receives
today. A protocol addition is required (§3.1.4).

**Second diff source — durable Goal approval artifacts.** For an `ApplyCode`
Goal approval the full diff is *already persisted durably* as an artifact:
`ApprovalSnapshot.artifacts` (`crates/fabric/src/types/approval.rs:200`) holds
`ApprovalArtifactRef { kind: "diff", relative_path, sha256 }`
(`approval.rs:110-115`; test fixture writes `coding-diffs/job.diff`,
`approval.rs:368-372`). The approval subject also carries scope axes:
`ApprovalSubject.allowed_scope: Vec<PathBuf>` and `apply_target: Option<PathBuf>`
(`approval.rs:88-89`), and `ApprovalCategory` has 9 variants — `ApplyCode`,
`ActivateGoal`, `SendMail`, `DeleteFile`, `ModifyCalendar`, `GitPush`,
`CapabilityExpansion`, `DaseinModification`, `BudgetExpansion`
(`approval.rs:33-43`) — plus `ApprovalRisk` (`approval.rs:47-52`). This is a
different path from the TUI's transient tool approval: `ApprovalNotice` carries
only `approval_id`, `tool`, `action_summary`, `risk_level`, and `detail`
(`crates/executive/src/application/turn_runtime_ports.rs:96-103`), and
`PendingApprovalRecord` stores only connection, tool, and responder
(`crates/executive/src/application/admin_service.rs:83-87`). The design must not
pretend that a durable `ApprovalSubject` is already available on that path.

### 1.3 The three concrete gaps

1. **No real diff.** `chat.rs:252-297` shows counts, not content. No highlight,
   no line numbers, no ± coloring, no per-line view. Codex renders each hunk with
   syntect highlighting, gutter line numbers, and GitHub-matched add/del tints
   (`codex-rs/tui/src/diff_render.rs`).
2. **Single-pane layout.** `draw.rs:44-64` is one vertical stack; the only ways
   to show detail are a whole-frame pager (`pager.rs`) or a centered modal. There
   is no persistent detail pane beside the transcript.
3. **Coarse approval.** Three decisions (`approval_dialog.rs:11-15`),
   14-line unstyled detail, no scroll, a broken wire mapping for the intended
   exact-tool session grant, and no way to say "allow writes *under this path*".
   The transient wire decision enum (`client.rs:199-203`) has no path-scoped
   choice, and its pending-record path has no typed mutation target today.
   Durable Goal approvals have category/path fields (`approval.rs:82-90`), but
   they cannot be silently reused as transient tool-approval state.

## 2. Goals / Non-goals

**Goals**

1. Render a syntax-highlighted unified diff (per-file, per-hunk, gutter line
   numbers, ± tints, dark/light-aware) in the TUI, reusing the existing `syntect`
   integration (`markdown.rs`).
2. Introduce a **middle detail pane** that hosts the diff (and, later, other
   inspectors) without breaking streaming or the existing overlays.
3. Extend transient approval to **scoped grants** — at minimum per-tool and
   per-path — with a keyboard-driven selector backed by a host-created typed
   scope subject and end-to-end enforcement.
4. Carry the diff text to the TUI within the 1 MB frame budget, with an
   out-of-band fetch fallback for oversized diffs.
5. Add snapshot coverage for the new renderers using the existing TUI snapshot
   harness.

**Non-goals**

- Mouse-driven diff selection or inline diff editing.
- A full theme-picker / live theme switching (codex has one; we expose only an
  explicit dark/light color mode with Dark default, §3.1.3).
- Word-level intra-line diff highlighting (codex does line-level; we match that).
- Re-architecting `StreamController`; the middle pane borrows from the chat
  flex region, it does not change the streaming model.
- Changing the durable approval contract semantics in `approval.rs` (one-time
  resolution, expiry, subject hashing stay exactly as-is).
- Per-agent MCP scoping. Executive changes required for artifact authorization
  and scoped session-grant enforcement are explicitly in scope.

## 3. Design

### 3.1 Feature 1 — Syntax-highlighted unified diff

#### 3.1.1 Where the diff text comes from (protocol)

**Option A — inline the rendered diff in `PatchDelta`.** Rejected as the sole
transport: it cannot retain the full artifact under the 1 MB frame cap.

**Option B — inline per-file structured hunks.** Add
`files_changed[i].hunks: Vec<{old_start,new_start,content}>` mirroring
`PatchHunk` (`structured_patch.rs:35-42`). Lets the TUI compute line numbers
precisely and re-highlight per file. More faithful, but duplicates the
structured-patch schema into the neutral DTO and is heavier on the wire.

**Option C — reference + paged fetch.** Send a content-addressed diff ref and a
bounded inline preview. Add `ClientRpcRequest::DiffArtifactGet { source,
offset, limit }`; each response page is independently capped, so no frame
"bypass" is required. This is the *only* option that
works for the approval path, where the diff is already a durable artifact
(`approval.rs:110-115`) and may be arbitrarily large.

**Locked decision: C as the authoritative transport, with a bounded preview for
fast rendering.**
- Tool results: during authorized application, capture successful file
  before/after content and render a canonical multi-file unified diff with
  `--- a/path`, `+++ b/path`, and hunk headers. Persist the complete bytes in the
  existing content-addressed Corpus `ArtifactStore`
  (`crates/corpus/src/tools/artifact/mod.rs:24-84`). `PatchDelta` carries
  `diff_artifact: Option<PatchDiffArtifactRef>` plus a UTF-8-safe preview capped
  at 64 KiB. It is never reconstructed from `StructuredPatchResult`.
- Durable Goal approvals: the diff is already persisted as an artifact
  (`ApprovalArtifactRef`, `approval.rs:110-115`); add
  `DiffArtifactGetSource::DurableApproval { approval_id, kind }`.
- Transient `apply_patch` approvals: `Tool` gains a default-none, host-only
  `approval_descriptor(input, context)` preflight. `ApplyPatchTool` implements it
  by parsing and validating the typed patch input before prompting, publishing
  the canonical preview to the same configured
  artifact root, and attaching
  `DiffArtifactGetSource::TransientApproval { approval_id, artifact_sha256 }`
  to the pending record and notification. Never treat arbitrary `detail` text or
  `action_summary` as a diff.
- Tool-result fetch uses `DiffArtifactGetSource::SessionToolResult { session_id,
  artifact_sha256 }`. Executive authorizes it only when the canonical Session
  contains that exact artifact ref for the authenticated principal. Approval
  fetch verifies approval ownership/status and validates the stored SHA-256.
  Transient approval fetch requires the authenticated principal, connection,
  and still-pending approval record to match.
- Both feed the same `DiffView` renderer (§3.1.2). The preview avoids a
  round-trip for small diffs; paged fetch provides the authoritative full bytes.

Trade-off accepted: three authorization sources, one content-addressed/paged read
contract and one renderer. Every page limit is clamped to 64 KiB and the client
revalidates the final digest before marking the full artifact loaded.

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

**Locked decision:** ship a `DiffTheme::{Dark, Light}` pair, selected by an
explicit TUI color-mode setting and defaulting to Dark. A later reliable terminal
background probe may feed `auto`, but D does not claim crossterm currently
provides one. Codex
resolves a `DiffTheme` per render and probes the active syntax theme
(`diff_render.rs:118`, `:191-227`); we take the simpler fixed-pair route (a full
theme picker is a non-goal). The syntect code theme stays `base16-ocean.dark` for
Dark and a light base16 for Light, matching `markdown.rs`.

#### 3.1.4 Wire changes summary

- `fabric::PatchDelta` (`tool.rs:117-121`): `+ diff_preview: Option<String>`,
  `+ diff_artifact: Option<PatchDiffArtifactRef>`, and
  `+ diff_preview_truncated: bool`, all serde-defaulted for compatibility.
  `PatchDeltaFileChange` also gains serde-defaulted `is_binary: bool`; binary
  rendering never depends on parsing prose markers.
- `apply_patch.rs`: collect authorized before/after content for successfully
  applied files, render canonical file headers/hunks, publish the full diff to
  the host-configured Corpus `ArtifactStore`, and populate the bounded
  preview/ref. Do not instantiate `OutputConfig::default()` as the production
  authority for this artifact root.
- `ClientRpcRequest` (`client.rs:55`): `+ DiffArtifactGet { source, offset,
  limit }`; page limit is clamped and Executive authorizes the source.

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

**Locked decision: A (horizontal split of the flex region), with B's overlay kept as
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
Narrow terminal (< 100 cols): detail pane collapses; `f` opens full-frame
overlay instead (reuses pager.rs render path).
```

### 3.3 Feature 3 — Granular approval scope

#### 3.3.1 Scope model

There are two approval domains and they remain separate:

- Durable Goal approvals retain `ApprovalCategory`, `allowed_scope`, and
  `apply_target` (`approval.rs:82-90`) unchanged.
- Transient tool approvals use `ApprovalRequest` from
  `crates/corpus/src/security/approval.rs:13-30`. They have a typed tool name and
  `WorkspacePolicy`, but no durable category or typed mutation target. The
  present `detail` is JSON text and is not authority.

Add `TransientApprovalScopeSubject { tool, canonical_write_roots,
subject_version, subject_sha256 }`. It is built server-side before the prompt:

1. `tool` comes from the registered tool identity, never from prose.
2. The additive `Tool::approval_descriptor` preflight extracts mutation targets
   from validated typed input. `ApplyPatchTool` reuses its parsed operations and
   `validate_mutation_path`; tools using the default implementation emit no path
   choices. The runner never branches on a tool name or parses prose.
3. Candidate roots are canonical ancestors inside both the invocation's actual
   targets and `ApprovalRequest.workspace.writable_roots()`, excluding protected
   paths. The client may choose only one exact advertised candidate.
4. The server hashes the typed subject and stores it in `PendingApprovalRecord`;
   `action_summary` and `detail` never participate in scope enforcement.

Extend `TransientApprovalDecision` (`client.rs:199-203`) while keeping the three
existing variants for protocol compatibility:

```rust
pub enum TransientApprovalDecision {
    Approve,             // once (unchanged)
    ApproveForSession,   // this exact tool in this thread (existing 'a' intent)
    Deny,                // (unchanged)
    ApprovePathForSession,   // NEW: writes whose resolved targets are under root
}
```

`ApprovalResponseParams` (`client.rs:216-218`) gains a typed optional
`scope_hint` containing the exact canonical path chosen from the server-provided
scope subject plus its version/hash. The exact tool always comes from the
pending server record. The client may select only an advertised path and may not
send an arbitrary broader tool/path.

Executive and Corpus enforcement land in the same change. A restart-safe
session grant table is keyed by authenticated principal, thread, exact tool,
optional canonical path root, subject version/hash, and expiry. Before executing
a write, the runner resolves actual mutation targets again and accepts a path
grant only when every target is under the granted root and outside protected
paths. The approval gate checks the narrowest matching valid grant before
creating a new prompt. Unknown variants, unsupported daemon schema, invalid
scope hints, stale subjects, and old peers fail closed to the existing three
choices. The TUI hides `p` unless daemon capability negotiation advertises
scoped approval support. **No scoped decision ever degrades to
`ApproveForSession`.**

Granularity chosen: **per-tool and per-path**. Per-command is deferred
(it needs command normalization the durable layer only stores as opaque
attributes today).

#### 3.3.2 Keyboard UX + dialog upgrade

Replace the flat 3-key modal (`approval_dialog.rs:54-61`) with a
selector-augmented modal, borrowing codex's `list_selection_view` +
`approval_overlay` pattern (`codex-rs/tui/src/bottom_pane/approval_overlay.rs`,
which distinguishes `CommandExecutionApprovalDecision` /
`FileChangeApprovalDecision` and a `GrantForSession` scope):

- `y` — approve once (unchanged).
- `a` — approve this exact **tool** for the current thread/session
  (`ApproveForSession`; existing intended semantics, clarified in the label).
- `p` — approve writes under this **path** for the session
  (`ApprovePathForSession`, shown only when the server provides a typed canonical
  path candidate).
- `n` / `d` / `Esc` — deny (unchanged).
- The hint row (`approval_dialog.rs:141-162`) lists only the scopes valid for the
  server-provided candidates (`p` is hidden when no typed path candidate exists;
  `a` remains available for compatibility).

`key_to_decision` (`approval_dialog.rs:54-61`) and the DialogDecision→Transient
mapping in `app/key_handler.rs` extend accordingly; `DialogDecision`
(`approval_dialog.rs:11-15`) gains the new path-scoped variant.

#### 3.3.3 Scrollable, highlighted approval detail

Today the detail pane is 14 lines, unstyled, unscrollable
(`approval_dialog.rs:73`, `:129-139`). Upgrade it to host a `DiffView` (§3.1.2)
when the approval carries an authorized transient or durable diff artifact,
fetched lazily via `DiffArtifactGet` (§3.1.1). Non-diff approvals keep a scrollable
plain-text detail (add `scroll` + `j/k` to the modal). This is where features 1
and 3 converge on one renderer.

## 4. Error handling

- **Oversized diffs (tool path):** producer stores the complete content-addressed
  artifact, truncates only the 64 KiB preview, and sets
  `diff_preview_truncated`; full view fetches 64 KiB pages. Every frame remains
  below the 1 MB cap (`transport.rs:8,53`).
- **Oversized diffs (approval path):** never inlined — fetched through the same
  paged `DiffArtifactGet` contract after ownership/hash validation.
- **Binary files:** the producer sets typed `is_binary` metadata and omits file
  contents from the text artifact. `DiffView` renders
  `⯄ binary file (N bytes → M bytes)` from that flag plus the existing
  `bytes_before`/`bytes_after` summary (`tool.rs:145-146`); no parser or
  highlighter is invoked for the file.
- **Missing highlight grammar:** if syntect has no syntax for the extension, fall
  back to plain (uncolored) diff lines with gutter + ± tint intact — exactly how
  `markdown.rs` degrades for unknown fences. Never fail the render.
- **Narrow terminals:** the horizontal split (§3.2.2) requires a min width
  (100 cols); below it, the detail pane is suppressed and `f` opens the
  full-frame overlay instead (reusing `pager.rs`), which already handles any
  width. The modal (§3.3.3) keeps its existing `popup_w` clamp
  (`approval_dialog.rs:72`).
- **Malformed diff text:** if `parse_unified_diff` errors, `DiffView` falls back
  to rendering the raw diff string as plain monospace (no crash), preserving the
  current worst-case behavior.
- **Missing artifact / fetch failure:** `DiffArtifactGet` returning an error
  leaves the modal on the plain-text summary and shows `diff unavailable`; the
  approval decision remains fully usable (fail-open on *display*, never on the
  decision).
- **Unsupported scoped approval:** hide scoped controls after capability
  negotiation. A forged/unsupported scoped response is rejected; it never
  becomes a broader session grant.

## 5. Verification

**Snapshot tests (extend the existing harness).** aletheon has one TUI snapshot
harness (`crates/interact/tests/tui_snapshots.rs`, using
`interact::tui::reducer::{reduce, snapshot_view}` and `AppState`) and a
`crates/interact/tests/snapshots/` directory. Add:
- `diff_view.rs` unit + snapshot: add/del/context coloring, gutter line numbers,
  multi-hunk separators, unknown-grammar fallback, binary-file case, truncation
  banner. (New module gets its own `#[cfg(test)]` + a `tui_snapshots.rs` case.)
- Layout snapshot: chat-only vs chat+detail split at wide width; collapse to
  chat-only at narrow width.
- Approval modal snapshot: `a` is labeled as an exact-tool grant; `p` appears
  only when the server advertises a typed canonical path candidate; scrollable
  diff detail.

**tmux / scenario tests.** Drive real key sequences through the existing
scaffolding:
- `tests/tui_tmux` — a scenario that submits a task producing an `apply_patch`
  result, presses `Ctrl+D` to open the diff pane, `f` to maximize, `j/k` to
  scroll, `Esc` to close.
- `tests/tui_scenarios` — an approval scenario asserting `a` sends the existing
  `ApproveForSession` exact-tool decision and `p` sends
  `ApprovePathForSession` with the advertised subject hash/path; include an
  old-daemon capability case in which only `p` is absent.

**Protocol/security tests.** Fabric round trips for default-absent PatchDelta
fields, paged artifact requests, capability negotiation, and scoped decisions;
Executive tests for owner/session binding, path canonicalization, expiry,
subject-version mismatch, digest mismatch, 64 KiB clamping, and proof that a
path-scoped decision can never create the broader tool-only
`ApproveForSession` grant.

**Commands (via the wrapper, narrowest first):**

```
bash scripts/cargo-agent.sh test -p interact
bash scripts/cargo-agent.sh test -p fabric
bash scripts/cargo-agent.sh test -p corpus apply_patch
bash scripts/cargo-agent.sh test -p executive approval
bash scripts/cargo-agent.sh fmt --all -- --check
```

**Installed-runtime acceptance (mandatory after implementation):** run
`sudo bash scripts/aletheon.sh deploy`; prove equal SHA-256 digests for
`target/release/aletheon`, `/usr/bin/aletheon`, and the executables behind the
machine and user daemon PIDs; sample both systemd restart counters twice and
confirm they do not increase; then complete three consecutive real TUI runs via
`/usr/bin/aletheon` and the official user socket. Each run must exercise a real
patch diff, full-artifact paging, and a real `ask apply_patch` path-scoped
approval choice, and must agree
across the rendered frame, canonical Session, approval/grant rows, and daemon
logs. Any rendered provider error or monitor/evidence disagreement fails the
run. Development binaries, alternate sockets, and isolated daemons are
diagnostic only.

## 6. Files touched

New:
- `crates/interact/src/tui/diff_view.rs` — `DiffView` renderer (parse via
  `structured_patch::parse_unified_diff`, syntect highlight reusing `markdown.rs`
  setup, gutter line numbers, dark/light themes).

Changed (interact):
- `crates/interact/Cargo.toml` — add the existing workspace `platform` crate for
  the canonical structured-patch parser; do not introduce a second parser.
- `crates/interact/src/host.rs:23-28` and
  `crates/interact/src/tui/term_compat.rs:6-102` — add typed dark/light color
  mode, Light palette, and Dark default.
- `crates/interact/src/tui/render/draw.rs:44-64` — horizontal split of the flex
  slot when `app.detail` is set; narrow-width guard.
- `crates/interact/src/tui/mod.rs:324,388` — add `detail: Option<DetailPane>`
  state beside `pending_approval`.
- `crates/interact/src/tui/app/key_handler.rs` — `Ctrl+D`/`f`/scroll keys;
  extend DialogDecision→`TransientApprovalDecision` mapping.
- `crates/interact/src/tui/approval_dialog.rs:11-15,54-61,129-162` — new
  path-scope variant, scope-aware hint row, scrollable/highlighted detail hosting
  `DiffView`.
- `crates/interact/src/tui/chat.rs:252-297` — keep the summary; add an
  "open diff (Ctrl+D)" affordance when `patch_delta.diff_preview` or
  `patch_delta.diff_artifact` is present.
- `crates/interact/src/tui/response.rs:317-353` — thread the diff artifact ref
  and typed scope subject into `pending_approval`; handle `DiffArtifactGet`
  pages and validate the completed artifact digest.
- `crates/interact/src/acp/mod.rs:34-57,161-166` — advertise client support and
  retain negotiated server support for diff paging and scoped approvals.
- `crates/interact/src/tui/mod.rs` (module list) — register `diff_view`.

Changed (fabric — additive neutral contracts):
- `crates/fabric/src/types/tool.rs:117-147,230-275` — `diff_preview`,
  `diff_artifact`, and `diff_preview_truncated` on `PatchDelta`; add the
  content-addressed `PatchDiffArtifactRef`, typed `is_binary` file metadata, and
  the default-none `Tool::approval_descriptor` preflight contract.
- `crates/fabric/src/protocol/client.rs:199-203,216-218` — one new path-scoped
  `TransientApprovalDecision` variant, typed
  `TransientApprovalScopeSubject`
  and `scope_hint`, `DiffArtifactGet` request/response, and explicit client/server
  capability bits.

Changed (corpus — artifact producer and final enforcement):
- `crates/corpus/src/tools/tools/apply_patch.rs:19-201,283-327` — accept the
  host-configured artifact store, snapshot authorized before/after bytes, render
  a canonical multi-file diff, persist it, and populate preview/ref.
- `crates/corpus/Cargo.toml` — add `similar = "2"`; use its line-based
  `TextDiff`/unified-diff formatter to render canonical before/after diffs for
  structured patches under the existing artifact-size budgets.
- `crates/corpus/src/security/approval.rs:13-38` — carry the server-built typed
  scope subject and optional diff artifact on transient requests.
- `crates/corpus/src/security/runner.rs:360-420,445-485` — resolve typed mutation
  targets through the tool preflight before prompting and again before honoring
  a path grant; honor host `RequireApproval` at every static permission level;
  never branch on tool name or use prose as scope evidence.
- `crates/corpus/src/tools/artifact/mod.rs:24-84` — add bounded, offset-based
  reads that verify the expected digest.

Changed (executive — authorization, persistence, and RPC):
- `crates/executive/src/application/admin_service.rs:73-260,531-565` — retain the
  typed scope/artifact subject in `PendingApprovalRecord`, validate scoped
  responses, and replace the current tool-only in-memory cache with the
  restart-safe session grant service.
- `crates/executive/src/application/approval/session_grant.rs` — new SQLite-backed
  grant repository with owner/thread/tool/path/version/hash/expiry keys and
  narrowest-match lookup.
- `crates/executive/src/application/turn_runtime_ports.rs:96-103` and
  `crates/executive/src/host/daemon/bootstrap/turn_runtime.rs:512-540` — carry
  typed scope candidates and diff refs into `ApprovalNotice`.
- `crates/executive/src/application/turn_pipeline.rs:976-999` — publish those
  server-authoritative fields on `approval_request`.
- `crates/executive/src/host/daemon/handler/rpc.rs:22-92` and
  `crates/executive/src/host/daemon/handler/rpc/rpc_admin.rs:114-143` — route
  paged diff reads and validate scoped approval responses against the pending
  subject and authenticated connection.
- `crates/executive/src/host/daemon/bootstrap/approval_gate.rs:28-82` — consult
  the session grant service, preserve fail-closed audit recording, and inject the
  canonical artifact store.
- `crates/executive/src/host/daemon/protocol.rs:1-115` and
  `crates/executive/src/host/daemon/server.rs:270-285` — negotiate the two new
  server capability bits.

Explicitly **unchanged:** `crates/interact/src/acp/transport.rs` (the framing
contract is honored, not modified); `crates/platform/src/structured_patch.rs`
(reused as-is); `crates/fabric/src/types/approval.rs` (durable contract unchanged).

## 7. Scope boundary

D spans `interact`, additive `fabric` contracts, the Corpus producer/enforcement
path, and Executive authorization/persistence/RPC. It shares Executive daemon
files with B (multi-agent planning), so the two workstreams must be sequenced or
assigned disjoint edits after an explicit ownership split. D is not complete
with client-only UX: scoped grants must be enforced end-to-end in the same wave,
and unsupported peers remain on y/a/n without any semantic downgrade.

D excludes: theme picker, mouse selection, word-level intra-line diff,
per-command scope, and changes to `StreamController`.

## Locked decisions

1. The authoritative diff transport is content-addressed reference + paged
   fetch, with a UTF-8-safe 64 KiB preview for tool results.
2. Every diff page is clamped to 64 KiB; the 1 MB framing cap is unchanged.
3. The primary layout is a 55/45 horizontal chat/detail split with full-frame
   overlay as the maximize path.
4. Below 100 columns, the split is hidden and only the full-frame overlay opens.
5. Per-tool and per-path grants land with Executive/Corpus enforcement in this
   wave; per-command grants are deferred. Path choices never degrade to the
   broader existing tool-only `ApproveForSession` behavior.
6. `a` remains the exact-tool shortcut and `p` is the new path shortcut; the
   hint row exposes `p` only for server-advertised candidates.
7. Dark/light themes use an explicit color-mode setting with Dark fallback; a
   live picker and unverified background probing remain out of scope.
