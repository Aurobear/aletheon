use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskCategory {
    ReadOnly,
    FileModification,
    SystemChange,
    Destructive,
}

impl RiskCategory {
    pub fn thresholds(&self) -> RiskThresholds {
        match self {
            Self::ReadOnly => RiskThresholds { same_call_threshold: 5, fail_streak_threshold: 7 },
            Self::FileModification => RiskThresholds { same_call_threshold: 3, fail_streak_threshold: 5 },
            Self::SystemChange => RiskThresholds { same_call_threshold: 2, fail_streak_threshold: 3 },
            Self::Destructive => RiskThresholds { same_call_threshold: 2, fail_streak_threshold: 2 },
        }
    }
}

#[derive(Debug, Clone)]
pub struct RiskThresholds {
    pub same_call_threshold: usize,
    pub fail_streak_threshold: usize,
}

pub struct RiskRule {
    pub tool_pattern: String,  // glob pattern
    pub category: RiskCategory,
}

pub struct RiskClassifier {
    rules: Vec<RiskRule>,
    default_category: RiskCategory,
}

impl RiskClassifier {
    pub fn with_defaults() -> Self {
        Self {
            rules: vec![
                RiskRule { tool_pattern: "file_read".into(), category: RiskCategory::ReadOnly },
                RiskRule { tool_pattern: "file_search".into(), category: RiskCategory::ReadOnly },
                RiskRule { tool_pattern: "system_status".into(), category: RiskCategory::ReadOnly },
                RiskRule { tool_pattern: "process_list".into(), category: RiskCategory::ReadOnly },
                RiskRule { tool_pattern: "memory_search".into(), category: RiskCategory::ReadOnly },
                RiskRule { tool_pattern: "file_write".into(), category: RiskCategory::FileModification },
                RiskRule { tool_pattern: "bash_exec".into(), category: RiskCategory::FileModification },
                RiskRule { tool_pattern: "core_memory_*".into(), category: RiskCategory::FileModification },
                RiskRule { tool_pattern: "service_*".into(), category: RiskCategory::SystemChange },
                RiskRule { tool_pattern: "package_*".into(), category: RiskCategory::SystemChange },
                RiskRule { tool_pattern: "network_*".into(), category: RiskCategory::SystemChange },
            ],
            default_category: RiskCategory::FileModification,
        }
    }

    pub fn classify(&self, tool_name: &str) -> RiskCategory {
        for rule in &self.rules {
            if glob_match(&rule.tool_pattern, tool_name) {
                return rule.category;
            }
        }
        self.default_category
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        text.starts_with(prefix)
    } else {
        pattern == text
    }
}
