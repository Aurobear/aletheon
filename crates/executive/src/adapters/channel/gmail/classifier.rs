//! Exact, bounded Gmail subject classification.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GmailClassification {
    Ask,
    Goal,
    Memory,
    Doc,
    Notification,
    Quarantine,
}

impl GmailClassification {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Goal => "goal",
            Self::Memory => "memory",
            Self::Doc => "doc",
            Self::Notification => "notification",
            Self::Quarantine => "quarantine",
        }
    }
}

pub fn classify_verified_subject(subject: &str) -> GmailClassification {
    if subject.len() > 998 || subject.contains(['\r', '\n', '\0']) {
        return GmailClassification::Quarantine;
    }
    for (prefix, classification) in [
        ("[ASK]", GmailClassification::Ask),
        ("[GOAL]", GmailClassification::Goal),
        ("[MEMORY]", GmailClassification::Memory),
        ("[DOC]", GmailClassification::Doc),
    ] {
        if subject == prefix
            || subject
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with(' '))
        {
            return classification;
        }
    }
    GmailClassification::Notification
}
