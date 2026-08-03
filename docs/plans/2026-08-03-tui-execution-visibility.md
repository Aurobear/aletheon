# TUI Execution Visibility Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make read-only diagnostics executable and present every tool action, terminal outcome, and runtime metric in a navigable TUI without exposing hidden reasoning.

**Architecture:** Keep `ClientEvent` and Session records authoritative. Improve the host command classifier in Corpus, derive bounded activity presentation from existing tool lifecycle events in Interact, and make selection/detail state local to the TUI. The layout remains one-column on narrow terminals and adds a selected activity detail pane on wide terminals.

**Tech Stack:** Rust, Tokio, Ratatui, Serde JSON, insta-style snapshot tests, repository Cargo wrapper.

---

### Task 1: Prove and fix read-only diagnostic classification

**Files:**
- Modify: `crates/corpus/src/security/command_effect.rs:34-212`
- Modify: `crates/corpus/src/security/runner.rs:221-243`
- Test: `crates/corpus/src/security/command_effect.rs` test module

- [ ] **Step 1: Add failing classification tests**

```rust
#[test]
fn diagnostic_help_and_systemctl_reads_are_read_only() {
    for command in [
        "/usr/bin/aletheon --help",
        "/usr/bin/aletheon memory --help",
        "/usr/bin/aletheon memory-agent -h",
        "systemctl --user status aletheon.service",
        "systemctl show aletheon-core.service -p ActiveState",
        "systemctl list-units --type=service",
    ] {
        assert_eq!(classify_command(command), CommandEffect::ReadOnly, "{command}");
    }
}

#[test]
fn systemctl_mutations_and_compound_help_fail_closed() {
    for command in [
        "systemctl restart aletheon.service",
        "systemctl --user enable --now aletheon.socket",
        "/usr/bin/aletheon --help; touch changed",
        "/usr/bin/aletheon --help | sh",
    ] {
        assert_ne!(classify_command(command), CommandEffect::ReadOnly, "{command}");
    }
}
```

- [ ] **Step 2: Run the focused test and confirm failure**

Run:

```bash
bash scripts/cargo-agent.sh test -p corpus security::command_effect -- --nocapture
```

Expected: the read-only `systemctl` and subcommand-help cases fail.

- [ ] **Step 3: Implement parsed read-only recognition**

Add helpers with these contracts:

```rust
fn is_read_only_systemctl(words: &[&str]) -> bool {
    let mut args = words.iter().copied().skip(1).filter(|word| {
        matches!(*word, "--user" | "--system" | "--no-pager" | "--plain" | "--quiet")
            == false
    });
    matches!(
        args.next(),
        Some("status" | "show" | "is-active" | "is-enabled" | "list-units" | "list-unit-files")
    )
}

fn is_help_probe(words: &[&str]) -> bool {
    words.len() >= 2
        && words.iter().skip(1).any(|word| matches!(*word, "--help" | "-h"))
        && words.iter().skip(1).all(|word| {
            !matches!(*word, "start" | "stop" | "restart" | "enable" | "disable" | "install" | "deploy")
        })
}
```

Call `is_read_only_systemctl` before the broad system-change rule and call
`is_help_probe` only after shell segments have been split and every segment can
be independently proven read-only. Preserve `SystemChange` for every other
`systemctl` form.

- [ ] **Step 4: Re-run tests**

Run the command from Step 2. Expected: PASS.

- [ ] **Step 5: Commit**

Commit subject: `fix(corpus): admit bounded read-only diagnostics` with a body
describing the false mutation classification and the fail-closed parsing rules.

### Task 2: Add selectable activity state and bounded tool presentation

**Files:**
- Modify: `crates/interact/src/tui/chat.rs:135-328,430-560`
- Modify: `crates/interact/src/tui/app/key_handler.rs:246-275`
- Modify: `crates/interact/src/tui/help_overlay.rs:65-85`
- Test: `crates/interact/tests/tui_snapshots.rs`

- [ ] **Step 1: Add failing tool activity tests**

Create tests that construct three `ExecEntry` values (success, policy denial,
execution error), navigate next/previous, and assert:

```rust
assert_eq!(chat.selected_exec_id(), Some("call-2"));
assert!(chat.toggle_selected_exec());
assert!(chat.selected_exec().unwrap().expanded);
assert_eq!(chat.selected_exec().unwrap().status(), ExecStatus::Denied);
```

Snapshot a completed entry and require the compact line to include the action,
status symbol, duration, and bounded argument summary.

- [ ] **Step 2: Run the focused tests and confirm failure**

```bash
bash scripts/cargo-agent.sh test -p interact --test tui_snapshots -- --nocapture
```

Expected: missing selection and status APIs.

- [ ] **Step 3: Implement presentation state**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecStatus { Running, Succeeded, Denied, Failed, Incomplete }

pub struct ExecEntry {
    // existing fields
    pub started_at: Option<fabric::MonoTime>,
    pub finished_at: Option<fabric::MonoTime>,
}

impl ExecEntry {
    pub fn status(&self) -> ExecStatus {
        if !self.finished { return ExecStatus::Running; }
        if self.is_policy_guidance() || self.output.starts_with("Policy denied:") {
            ExecStatus::Denied
        } else if self.is_error { ExecStatus::Failed } else { ExecStatus::Succeeded }
    }
}
```

Add `selected_exec_id: Option<String>` to `ChatWidget` and methods
`select_next_exec`, `select_previous_exec`, `selected_exec`,
`toggle_selected_exec`, and `selected_exec_detail`. Selection must survive new
stream events and clear only when the selected entry no longer exists.

Bind `Alt+Up/Alt+Down` to activity navigation, `Ctrl+B` to selected expansion
with last-entry fallback, and `Enter` to expansion only while activity selection
mode is active. Document all bindings in `/help`.

- [ ] **Step 4: Re-run the focused tests**

Run Step 2. Expected: PASS.

- [ ] **Step 5: Commit**

Commit subject: `feat(tui): make tool activity navigable` with selection,
bounded output, denial state, and compatibility-key details in the body.

### Task 3: Render an activity detail pane and responsive hierarchy

**Files:**
- Create: `crates/interact/src/tui/activity_detail.rs`
- Modify: `crates/interact/src/tui/mod.rs`
- Modify: `crates/interact/src/tui/render/draw.rs:19-93`
- Modify: `crates/interact/src/tui/render/header.rs:10-43`
- Modify: `crates/interact/src/tui/render/mod.rs`
- Test: `crates/interact/tests/tui_snapshots.rs`

- [ ] **Step 1: Add failing wide/narrow layout snapshots**

Use Ratatui `TestBackend` at `120x36` and `78x24`. The wide snapshot must contain
`Activity detail`, `Arguments`, `Result`, and `Status`; the narrow snapshot must
contain the compact tool row without clipping the input or footer.

- [ ] **Step 2: Run snapshots and confirm failure**

```bash
bash scripts/cargo-agent.sh test -p interact --test tui_snapshots activity -- --nocapture
```

Expected: detail labels absent.

- [ ] **Step 3: Implement the detail widget**

Define:

```rust
pub struct ActivityDetail<'a> { pub entry: &'a ExecEntry, pub caps: &'a TermCaps }

impl Widget for ActivityDetail<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // bordered title, status, bounded arguments, bounded result, call id
    }
}
```

At widths `>= 100`, split the conversation area 62/38 when an activity is
selected. Below 100 columns, keep the conversation full width and render detail
as an overlay only after the user expands the selected entry. Never reserve an
empty right panel.

Render the header as two information rows when height permits:

```text
Aletheon · <agent/mode> · <effective model>
<provider> · <connection> · ctx <used>/<max> · cache <reported|unknown>
```

- [ ] **Step 4: Re-run snapshots**

Run Step 2. Expected: PASS with reviewed snapshots at both sizes.

- [ ] **Step 5: Commit**

Commit subject: `feat(tui): add responsive execution detail` with the responsive
breakpoint and no-empty-panel behavior described in the body.

### Task 4: Add authoritative per-turn activity counters and safe progress

**Files:**
- Modify: `crates/interact/src/tui/state.rs:108-188`
- Modify: `crates/interact/src/tui/response.rs:97-166,430-510`
- Modify: `crates/interact/src/tui/status.rs:81-209`
- Modify: `crates/interact/src/tui/streaming.rs:87-111,250-299`
- Test: `crates/interact/tests/tui_reducer.rs`
- Test: `crates/interact/tests/tui_snapshots.rs`

- [ ] **Step 1: Add failing counter and privacy tests**

Feed one successful, one denied, and one failed tool lifecycle plus four
inference receipts into the reducer/event handler. Assert separate values:

```rust
assert_eq!(state.turn_activity.inference_rounds, 4);
assert_eq!(state.turn_activity.tool_calls, 3);
assert_eq!(state.turn_activity.succeeded, 1);
assert_eq!(state.turn_activity.denied, 1);
assert_eq!(state.turn_activity.failed, 1);
assert!(!transcript.contains("private chain of thought"));
```

- [ ] **Step 2: Run focused tests and confirm failure**

```bash
bash scripts/cargo-agent.sh test -p interact --test tui_reducer -- --nocapture
bash scripts/cargo-agent.sh test -p interact --test tui_snapshots status -- --nocapture
```

Expected: missing `turn_activity` state and status content.

- [ ] **Step 3: Implement counters and typed summaries**

Add:

```rust
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TurnActivity {
    pub inference_rounds: usize,
    pub provider_retries: usize,
    pub tool_calls: usize,
    pub succeeded: usize,
    pub denied: usize,
    pub failed: usize,
}
```

Reset it on `TurnStarted`; update it only from authoritative inference/tool
events. Render:

```text
cwd · infer 4 · retry 0 · tools 1/3 · denied 1 · failed 1 · 12.4s · ctx 9k/1m
```

Keep cumulative billed tokens and cache usage separate. Replace the completed
thinking placeholder with `Reasoning completed · <duration>` but continue to
clear the raw buffer and never persist it. Tool lifecycle rows are the safe
progress summary; do not synthesize model rationale.

- [ ] **Step 4: Re-run focused tests**

Run Step 2. Expected: PASS.

- [ ] **Step 5: Commit**

Commit subject: `feat(tui): summarize authoritative turn activity` with metric
separation and hidden-reasoning privacy in the body.

### Task 5: Require evidence-qualified runtime conclusions

**Files:**
- Modify: `crates/cognit/prompts/default_system.md`
- Test: `crates/cognit/src/config/mod.rs:800-835`

- [ ] **Step 1: Add a failing prompt contract assertion**

Assert that the system prompt includes all three evidence labels:
`documented design`, `observed runtime fact`, and `unverified inference`, plus a
requirement to disclose denied or failed probes.

- [ ] **Step 2: Run the narrow prompt test and confirm failure**

```bash
bash scripts/cargo-agent.sh test -p cognit prompt_contract -- --nocapture
```

Expected: missing evidence-qualification clause.

- [ ] **Step 3: Add the minimal prompt rule**

Append one compact paragraph requiring runtime answers to distinguish documented
design, observed runtime facts, and unverified inference, and to disclose any
failed or denied probe that limits the conclusion. Do not add task-specific,
language-specific, GBrain-specific, or acceptance-prompt-specific behavior.

- [ ] **Step 4: Re-run the prompt test**

Run Step 2. Expected: PASS.

- [ ] **Step 5: Commit**

Commit subject: `fix(cognit): qualify conclusions by evidence level` with the
unsupported-runtime-claim problem in the body.

### Task 6: Integrated validation and installed-runtime acceptance

**Files:**
- Modify only if validation exposes defects in files owned by Tasks 1-5.

- [ ] **Step 1: Run formatting and focused suites**

```bash
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh test -p corpus security::command_effect -- --nocapture
bash scripts/cargo-agent.sh test -p interact --test tui_reducer -- --nocapture
bash scripts/cargo-agent.sh test -p interact --test tui_snapshots -- --nocapture
bash scripts/cargo-agent.sh test -p cognit prompt_contract -- --nocapture
```

Expected: all PASS.

- [ ] **Step 2: Review the complete diff**

```bash
git diff --check
git status --short
git log --oneline --decorate -8
```

Expected: no whitespace errors and only scoped files changed.

- [ ] **Step 3: Deploy the system runtime**

```bash
sudo bash scripts/aletheon.sh deploy
```

Expected: deployment exits zero; release, `/usr/bin/aletheon`, machine daemon,
and user daemon executable SHA-256 digests match.

- [ ] **Step 4: Run real-TUI acceptance**

In one unchanged `/usr/bin/aletheon` TUI session, run a task that executes
`/usr/bin/aletheon memory --help`, `systemctl --user show aletheon.service`, and
a known denied mutating systemctl command. Verify the rendered frame shows all
three tool cards, selectable details, success/denial states, separated activity
and final answer, and no raw hidden reasoning. Then run two follow-up turns and
verify the prompt returns each time.

- [ ] **Step 5: Record acceptance evidence**

Record source commit, installed digests, session ID, rendered final frame,
session DB location, inference rounds, retries, tool counts, active context,
cache usage, daemon logs, and stable restart counters in the implementation
report. Do not report deployment acceptance if the deploy command or any real
TUI turn fails.
