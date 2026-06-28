use std::collections::HashMap;
use std::path::PathBuf;

/// Layer 1: Per-tool capture limits (process-level).
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            max_stdout_bytes: 1_048_576,
            max_stderr_bytes: 1_048_576,
        }
    }
}

/// Layer 2: Per-result output processing configuration.
#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub max_output_chars: usize,
    pub overflow_dir: PathBuf,
    pub truncation: TruncationPolicy,
    pub tool_overrides: HashMap<String, usize>,
    pub pinned_thresholds: HashMap<String, usize>,
    pub retention_days: u32,
}

#[derive(Debug, Clone)]
pub struct TruncationPolicy {
    pub head_lines: usize,
    pub tail_lines: usize,
    pub max_bytes: Option<usize>,
}

impl Default for TruncationPolicy {
    fn default() -> Self {
        Self {
            head_lines: 50,
            tail_lines: 20,
            max_bytes: None,
        }
    }
}

impl Default for OutputConfig {
    fn default() -> Self {
        let mut pinned = HashMap::new();
        pinned.insert("file_read".to_string(), usize::MAX);

        Self {
            max_output_chars: 100_000,
            overflow_dir: PathBuf::from("/tmp/agentd/overflow"),
            truncation: TruncationPolicy::default(),
            tool_overrides: HashMap::new(),
            pinned_thresholds: pinned,
            retention_days: 7,
        }
    }
}

/// Layer 3: Per-turn aggregate budget configuration.
#[derive(Debug, Clone)]
pub struct TurnBudgetConfig {
    pub turn_budget_chars: usize,
    pub preview_chars: usize,
}

impl Default for TurnBudgetConfig {
    fn default() -> Self {
        Self {
            turn_budget_chars: 200_000,
            preview_chars: 1_500,
        }
    }
}
