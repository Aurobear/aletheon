//! StreamController: two-region streaming with bounded tail.
//!
//! Inspired by Codex's StreamController:
//! - Stable region: committed content in scrollback (immutable)
//! - Tail region: currently-streaming content (mutable, real-time)

use std::time::Instant;

const THINKING_VIEW_MAX: usize = 4096; // 4KB cap for thinking tail
const THINKING_TAIL_LINES: usize = 12; // max visual lines for thinking

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
    thinking_start: Option<Instant>,
    /// Thinking collapsed state
    thinking_collapsed: bool,
}

impl StreamController {
    pub fn new() -> Self {
        Self {
            committed: String::new(),
            tail: String::new(),
            thinking_buf: String::new(),
            thinking: false,
            thinking_start: None,
            thinking_collapsed: true,
        }
    }

    pub fn start_turn(&mut self) {
        self.committed.clear();
        self.tail.clear();
        self.thinking_buf.clear();
        self.thinking = false;
        self.thinking_start = None;
    }

    pub fn push_thinking(&mut self, text: &str) {
        if !self.thinking {
            self.thinking = true;
            self.thinking_start = Some(Instant::now());
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

    pub fn current_text(&self) -> String {
        let mut result = String::new();
        if self.thinking && !self.thinking_collapsed {
            result.push_str(&self.format_thinking_expanded());
        }
        result.push_str(&self.committed);
        result.push_str(&self.tail);
        result
    }

    pub fn commit(&mut self) {
        if self.thinking {
            self.commit_thinking();
        }
        self.committed.push_str(&self.tail);
        self.tail.clear();
    }

    pub fn toggle_thinking(&mut self) {
        self.thinking_collapsed = !self.thinking_collapsed;
    }

    pub fn thinking_elapsed(&self) -> Option<f64> {
        self.thinking_start.map(|s| s.elapsed().as_secs_f64())
    }

    pub fn is_thinking(&self) -> bool {
        self.thinking
    }

    pub fn thinking_collapsed(&self) -> bool {
        self.thinking_collapsed
    }

    fn commit_thinking(&mut self) {
        if let Some(start) = self.thinking_start {
            let elapsed = start.elapsed().as_secs_f64();
            if self.thinking_collapsed {
                self.committed.push_str(&format!("✻ Thought for {:.1}s\n\n", elapsed));
            } else {
                self.committed.push_str(&self.format_thinking_expanded());
            }
        }
        self.thinking = false;
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
            result.push_str(&format!("│ {}\n", line));
        }
        result.push_str(&format!("({:.1}s)\n\n", elapsed));
        result
    }
}
