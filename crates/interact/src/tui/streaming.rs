//! StreamController: two-region streaming with bounded tail.
//!
//! Inspired by Codex's StreamController:
//! - Stable region: committed content in scrollback (immutable)
//! - Tail region: currently-streaming content (mutable, real-time)

use fabric::{Clock, MonoTime};
use std::sync::Arc;

const THINKING_VIEW_MAX: usize = 4096; // 4KB cap for thinking tail
const THINKING_TAIL_LINES: usize = 12; // max visual lines for thinking

// ---------------------------------------------------------------------------
// Table holdback helpers
// ---------------------------------------------------------------------------

fn is_pipe_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|')
}

fn is_separator_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|')
        && trimmed.ends_with('|')
        && trimmed
            .chars()
            .all(|c| c == '|' || c == '-' || c == ':' || c == ' ')
}

// ---------------------------------------------------------------------------
// Table holdback state
// ---------------------------------------------------------------------------

enum TableHoldbackState {
    /// No table currently being tracked.
    None,
    /// Saw a pipe row, waiting to see if it is a table header.
    WaitingForSeparator { header_line_idx: usize },
    /// Confirmed table (header + separator seen), accumulating rows.
    Accumulating { header_end_idx: usize },
}

// ---------------------------------------------------------------------------
// StreamController
// ---------------------------------------------------------------------------

pub struct StreamController {
    /// Committed text (stable, in scrollback)
    committed: String,
    /// Current streaming text (tail, mutable)
    tail: String,
    /// Thinking buffer (bounded)
    thinking_buf: String,
    /// Whether currently in thinking phase
    thinking: bool,
    /// Thinking start time
    thinking_start: Option<MonoTime>,
    /// Thinking collapsed state
    thinking_collapsed: bool,
    /// Table holdback state for flicker-free markdown table streaming
    table_holdback: TableHoldbackState,
    /// Clock for time-based operations
    clock: Arc<dyn Clock>,
}

impl Default for StreamController {
    fn default() -> Self {
        Self::new(Arc::new(crate::tui::host_time::ClientClock::new()))
    }
}

impl StreamController {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            committed: String::new(),
            tail: String::new(),
            thinking_buf: String::new(),
            thinking: false,
            thinking_start: None,
            thinking_collapsed: true,
            table_holdback: TableHoldbackState::None,
            clock,
        }
    }

    pub fn start_turn(&mut self) {
        self.committed.clear();
        self.tail.clear();
        self.thinking_buf.clear();
        self.thinking = false;
        self.thinking_start = None;
        // Match Codex's presentation: reasoning opens while it is streaming,
        // then returns to a collapsed summary when the turn completes.
        self.thinking_collapsed = true;
        self.table_holdback = TableHoldbackState::None;
    }

    pub fn push_thinking(&mut self, text: &str) {
        if !self.thinking {
            self.thinking = true;
            self.thinking_start = Some(self.clock.mono_now());
            self.thinking_collapsed = false;
        }
        self.thinking_buf.push_str(text);
        // Bounded tail: keep only last THINKING_VIEW_MAX bytes
        if self.thinking_buf.len() > THINKING_VIEW_MAX {
            let excess = self.thinking_buf.len() - THINKING_VIEW_MAX;
            self.thinking_buf.drain(..excess);
        }
    }

    pub fn push_text(&mut self, text: &str) {
        // If we were thinking, commit thinking summary
        if self.thinking {
            self.commit_thinking();
        }
        self.tail.push_str(text);
    }

    pub fn current_text(&mut self) -> String {
        let mut result = String::new();
        if self.thinking && !self.thinking_collapsed {
            result.push_str(&self.format_thinking_expanded());
        }
        result.push_str(&self.committed);

        // Table holdback: detect pipe tables in the tail and hold them back
        // until they are complete, preventing row-by-row flicker.
        let tail_lines: Vec<String> = self.tail.lines().map(|s| s.to_string()).collect();
        let tail_line_refs: Vec<&str> = tail_lines.iter().map(|s| s.as_str()).collect();
        self.update_table_holdback(&tail_line_refs);

        match self.table_holdback {
            TableHoldbackState::None | TableHoldbackState::WaitingForSeparator { .. } => {
                result.push_str(&self.tail);
            }
            TableHoldbackState::Accumulating { header_end_idx } => {
                // Only emit lines before the held-back table.
                if header_end_idx > 0 && header_end_idx <= tail_lines.len() {
                    for (i, line) in tail_lines.iter().take(header_end_idx).enumerate() {
                        if i > 0 {
                            result.push('\n');
                        }
                        result.push_str(line);
                    }
                }
                // When header_end_idx == 0 the table starts at the very
                // beginning of the tail — show nothing from tail.
            }
        }

        result
    }

    /// Scan the tail lines and update the table holdback state machine.
    fn update_table_holdback(&mut self, tail_lines: &[&str]) {
        match self.table_holdback {
            TableHoldbackState::None => {
                for (i, line) in tail_lines.iter().enumerate() {
                    if is_pipe_row(line) {
                        if i + 1 < tail_lines.len() && is_separator_row(tail_lines[i + 1]) {
                            // Header + separator found. Check whether the table
                            // is already complete (non-pipe, non-empty line
                            // after at least one data row).
                            let data_start = i + 2;
                            let complete = tail_lines.iter().enumerate().any(|(j, l)| {
                                j > data_start
                                    && !l.trim().is_empty()
                                    && !is_pipe_row(l)
                                    && !is_separator_row(l)
                            });
                            if complete {
                                break; // complete table, no holdback needed
                            }
                            self.table_holdback =
                                TableHoldbackState::Accumulating { header_end_idx: i };
                            return;
                        } else if i + 1 >= tail_lines.len() {
                            // Only the header has arrived so far — the
                            // separator may come in the next delta.
                            self.table_holdback =
                                TableHoldbackState::WaitingForSeparator { header_line_idx: i };
                            return;
                        }
                        // i+1 exists but is not a separator: false positive, skip.
                    }
                }
            }
            TableHoldbackState::WaitingForSeparator { header_line_idx } => {
                let h = header_line_idx;
                if h + 1 < tail_lines.len() {
                    if is_separator_row(tail_lines[h + 1]) {
                        // Header + separator confirmed.
                        let data_start = h + 2;
                        let complete = tail_lines.iter().enumerate().any(|(j, l)| {
                            j > data_start
                                && !l.trim().is_empty()
                                && !is_pipe_row(l)
                                && !is_separator_row(l)
                        });
                        if complete {
                            self.table_holdback = TableHoldbackState::None;
                        } else {
                            self.table_holdback =
                                TableHoldbackState::Accumulating { header_end_idx: h };
                        }
                    } else if !is_pipe_row(tail_lines[h + 1]) {
                        // The line after the header is neither a separator nor a
                        // pipe row — false positive.
                        self.table_holdback = TableHoldbackState::None;
                    }
                    // If h+1 is also a pipe row but not a separator: unlikely
                    // in GFM tables; keep waiting — the separator may appear
                    // between two pipe rows in unusual formats.
                }
            }
            TableHoldbackState::Accumulating { header_end_idx } => {
                if header_end_idx + 2 < tail_lines.len() {
                    let data_start = header_end_idx + 2;
                    let should_release = tail_lines.iter().enumerate().any(|(j, l)| {
                        j > data_start
                            && !l.trim().is_empty()
                            && !is_pipe_row(l)
                            && !is_separator_row(l)
                    });
                    if should_release {
                        self.table_holdback = TableHoldbackState::None;
                    }
                }
            }
        }
    }

    pub fn commit(&mut self) {
        if self.thinking {
            self.commit_thinking();
        }
        self.committed.push_str(&self.tail);
        self.tail.clear();
        self.table_holdback = TableHoldbackState::None;
    }

    pub fn toggle_thinking(&mut self) {
        self.thinking_collapsed = !self.thinking_collapsed;
    }

    pub fn thinking_elapsed(&self) -> Option<f64> {
        self.thinking_start.map(|s| {
            let elapsed_ms = self.clock.mono_now().0.saturating_sub(s.0);
            elapsed_ms as f64 / 1000.0
        })
    }

    pub fn is_thinking(&self) -> bool {
        self.thinking
    }

    pub fn thinking_collapsed(&self) -> bool {
        self.thinking_collapsed
    }

    fn commit_thinking(&mut self) {
        if let Some(start) = self.thinking_start {
            let elapsed_ms = self.clock.mono_now().0.saturating_sub(start.0);
            let elapsed = elapsed_ms as f64 / 1000.0;
            self.committed
                .push_str(&format!("✻ Thought for {elapsed:.1}s\n\n"));
        }
        self.thinking = false;
        self.thinking_collapsed = true;
        self.thinking_buf.clear();
        self.thinking_start = None;
    }

    fn format_thinking_expanded(&self) -> String {
        let elapsed = self.thinking_elapsed().unwrap_or(0.0);
        let lines: Vec<&str> = self.thinking_buf.lines().collect();
        let display_lines: &[&str] = if lines.len() > THINKING_TAIL_LINES {
            &lines[lines.len() - THINKING_TAIL_LINES..]
        } else {
            &lines
        };
        let mut result = String::from("✻ Thinking...\n");
        for line in display_lines {
            result.push_str(&format!("│ {line}\n"));
        }
        result.push_str(&format!("({elapsed:.1}s)\n\n"));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::WallTime;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Default)]
    struct TestClock {
        mono_ms: AtomicU64,
    }

    impl TestClock {
        fn advance(&self, millis: u64) {
            self.mono_ms.fetch_add(millis, Ordering::SeqCst);
        }
    }

    impl Clock for TestClock {
        fn wall_now(&self) -> WallTime {
            WallTime(0)
        }

        fn mono_now(&self) -> MonoTime {
            MonoTime(self.mono_ms.load(Ordering::SeqCst))
        }
    }

    #[test]
    fn thinking_is_expanded_while_streaming_then_collapses_before_answer() {
        let clock = Arc::new(TestClock::default());
        let mut stream = StreamController::new(clock.clone());

        stream.start_turn();
        stream.push_thinking("先分析问题");

        assert!(!stream.thinking_collapsed());
        assert!(stream.current_text().contains("先分析问题"));

        clock.advance(1_250);
        stream.push_text("最终回答");

        let rendered = stream.current_text();
        assert!(stream.thinking_collapsed());
        assert!(rendered.contains("✻ Thought for 1.2s"));
        assert!(!rendered.contains("先分析问题"));
        assert!(rendered.ends_with("最终回答"));
    }

    #[test]
    fn completed_thinking_only_turn_is_collapsed() {
        let clock = Arc::new(TestClock::default());
        let mut stream = StreamController::new(clock.clone());

        stream.start_turn();
        stream.push_thinking("内部推理");
        clock.advance(500);
        stream.commit();

        assert_eq!(stream.current_text(), "✻ Thought for 0.5s\n\n");
        assert!(stream.thinking_collapsed());
        assert!(!stream.is_thinking());
    }

    #[test]
    fn new_turn_restores_codex_default_even_after_manual_toggle() {
        let clock = Arc::new(TestClock::default());
        let mut stream = StreamController::new(clock);

        stream.start_turn();
        stream.toggle_thinking();
        assert!(!stream.thinking_collapsed());

        stream.start_turn();
        assert!(stream.thinking_collapsed());
    }
}
