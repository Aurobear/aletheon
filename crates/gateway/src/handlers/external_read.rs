//! External-source read preflight capability.
//!
//! Detects bounded read-only external-source intents (e.g. "today's events", "unread
//! mail") arriving as chat text and either asks the user to pick an account
//! (no LLM involved) or rewrites the query with a
//! `<trusted-external-account>` marker before the normal turn executor runs.
//!
//! This module is the *only* place in `channel/` allowed to know about
//! providers. It is wired into [`super::chat::ChatHandler`] through the neutral
//! [`ChatPreprocessor`] hook, and only when a external-source integration is
//! configured (see `bootstrap/channels.rs`). `dispatcher.rs` and
//! `telegram/mod.rs` must stay free of any reference to it.

use std::sync::Arc;

use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Neutral hook (implemented by this module; consumed by `ChatHandler`)
// ---------------------------------------------------------------------------

/// Preprocessing hook a `ChatHandler` runs before the turn executor.
#[async_trait]
pub trait ChatPreprocessor: Send + Sync {
    async fn preprocess(&self, principal: &str, text: &str) -> anyhow::Result<ChatPreprocess>;
}

/// Outcome of a [`ChatPreprocessor`] pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatPreprocess {
    /// Short-circuit with this reply text; the turn executor never runs.
    Reply(String),
    /// Run the turn executor with this rewritten query instead of the
    /// original text.
    Rewrite(String),
    /// No preprocessing needed; run the turn executor with the original
    /// text.
    Passthrough,
}

// ---------------------------------------------------------------------------
// external account directory
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ExternalAccountDirectory: Send + Sync {
    async fn active_account_labels(&self, principal: &str) -> anyhow::Result<Vec<String>>;
}

// ---------------------------------------------------------------------------
// ExternalReadPreprocessor
// ---------------------------------------------------------------------------

/// [`ChatPreprocessor`] that gates bounded read-only external-source intents behind
/// an explicit account choice (multiple active accounts) or wraps the query
/// with a `<trusted-external-account>` marker (exactly one active account).
/// Non-External-source read text passes through unchanged.
pub struct ExternalReadPreprocessor {
    accounts: Arc<dyn ExternalAccountDirectory>,
}

impl ExternalReadPreprocessor {
    pub fn new(accounts: Arc<dyn ExternalAccountDirectory>) -> Self {
        Self { accounts }
    }
}

#[async_trait]
impl ChatPreprocessor for ExternalReadPreprocessor {
    async fn preprocess(&self, principal: &str, text: &str) -> anyhow::Result<ChatPreprocess> {
        if !is_external_read_query(text) {
            return Ok(ChatPreprocess::Passthrough);
        }
        let labels = self.accounts.active_account_labels(principal).await?;
        if labels.len() > 1 {
            return Ok(ChatPreprocess::Reply(account_choice_prompt(&labels)));
        }
        if let Some(label) = labels.first() {
            return Ok(ChatPreprocess::Rewrite(selected_account_context(
                label, text,
            )));
        }
        Ok(ChatPreprocess::Passthrough)
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (moved from `telegram/mod.rs`)
// ---------------------------------------------------------------------------

/// Detect the bounded read-only external-source intents that need an explicit account
/// selection before entering the normal ReAct pipeline.
pub(crate) fn is_external_read_query(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    [
        "today's events",
        "today’s events",
        "today events",
        "important unread",
        "unread mail",
        "unread email",
        "今天的日程",
        "今日事件",
        "重要未读",
        "未读邮件",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(crate) fn account_choice_prompt(labels: &[String]) -> String {
    let choices = labels
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, label)| format!("{}. {}", index + 1, truncate_label(label)))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Please choose an external account before I run this read-only query:\n{choices}")
}

pub(crate) fn selected_account_context(label: &str, query: &str) -> String {
    format!(
        "<trusted-external-account>{}</trusted-external-account>\n{}",
        truncate_label(label),
        query
    )
}

fn truncate_label(label: &str) -> String {
    label.chars().take(128).collect()
}
