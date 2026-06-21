use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Test infrastructure ─────────────────────────────────────────

/// Configuration for test mode, passed from CLI flags.
#[derive(Default)]
pub struct TestConfig {
    pub test_input: Option<PathBuf>,
    pub record_frames: Option<PathBuf>,
    pub record_events: Option<PathBuf>,
    pub auto_submit: bool,
    pub test_timeout: u64,
}

/// Milliseconds since epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── FrameRecorder ───────────────────────────────────────────────

/// Snapshot of a single rendered frame.
#[derive(serde::Serialize)]
pub struct FrameSnapshot {
    pub ts: u64,
    pub cols: u16,
    pub rows: u16,
    pub content: String,
    pub thinking_visible: bool,
    pub tool_count: usize,
}

/// Writes a JSONL snapshot after each render.
pub struct FrameRecorder {
    file: fs::File,
}

impl FrameRecorder {
    pub fn new(path: &std::path::Path) -> anyhow::Result<Self> {
        let file = fs::File::create(path)?;
        Ok(Self { file })
    }

    pub fn write(&mut self, snapshot: &FrameSnapshot) {
        if let Ok(line) = serde_json::to_string(snapshot) {
            let _ = writeln!(self.file, "{}", line);
        }
    }
}

/// Extract visible text from a ratatui Buffer (one line per row).
pub fn buffer_to_text(buffer: &ratatui::buffer::Buffer) -> String {
    let area = buffer.area;
    let mut lines = Vec::with_capacity(area.height as usize);
    for y in area.y..area.y + area.height {
        let mut line = String::new();
        for x in area.x..area.x + area.width {
            let cell = &buffer[(x, y)];
            line.push_str(cell.symbol());
        }
        lines.push(line);
    }
    lines.join("\n")
}

// ── EventRecorder ───────────────────────────────────────────────

/// Writes one JSONL line per daemon->TUI event.
pub struct EventRecorder {
    file: fs::File,
}

impl EventRecorder {
    pub fn new(path: &std::path::Path) -> anyhow::Result<Self> {
        let file = fs::File::create(path)?;
        Ok(Self { file })
    }

    pub fn write(&mut self, event_json: &serde_json::Value) {
        let record = serde_json::json!({
            "ts": now_ms(),
            "type": event_json.get("type").and_then(|v| v.as_str()).unwrap_or(""),
            "params": event_json,
        });
        if let Ok(line) = serde_json::to_string(&record) {
            let _ = writeln!(self.file, "{}", line);
        }
    }
}

// ── TestInputReader ─────────────────────────────────────────────

/// Reads lines from a test input file and optionally auto-submits them.
pub struct TestInputReader {
    lines: Vec<String>,
    index: usize,
    pub auto_submit: bool,
    /// All lines consumed and the final turn_done received.
    pub done: bool,
}

impl TestInputReader {
    pub fn new(path: &std::path::Path, auto_submit: bool) -> anyhow::Result<Self> {
        let file = fs::File::open(path)?;
        let reader = io::BufReader::new(file);
        let lines: Vec<String> = reader
            .lines()
            .map_while(Result::ok)
            .collect();
        Ok(Self {
            lines,
            index: 0,
            auto_submit,
            done: false,
        })
    }

    /// Returns the next line to submit, or None if exhausted.
    pub fn next_line(&mut self) -> Option<String> {
        if self.index < self.lines.len() {
            let line = self.lines[self.index].clone();
            self.index += 1;
            Some(line)
        } else {
            None
        }
    }

    /// Called when a turn completes; returns next line if auto_submit.
    pub fn on_turn_done(&mut self) -> Option<String> {
        if self.auto_submit {
            let next = self.next_line();
            if next.is_none() {
                self.done = true;
            }
            next
        } else {
            if self.index >= self.lines.len() {
                self.done = true;
            }
            None
        }
    }

    /// Whether all input lines have been consumed.
    pub fn is_exhausted(&self) -> bool {
        self.index >= self.lines.len()
    }
}
