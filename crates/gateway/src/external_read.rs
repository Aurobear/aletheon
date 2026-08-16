//! External-source read preflight capability.
//!
//! Detects bounded read-only external-source intents (e.g. "today's events", "unread
//! mail") arriving as chat text and either asks the user to pick an account
//! (no LLM involved) or rewrites the query with a
//! `<trusted-external-account>` marker before the normal turn executor runs.
//!
//! This module is the *only* place in `channel/` allowed to know about
//! providers. It is wired into [`super::chat::ChatHandler`] through the typed
//! read Application port, and only when a external-source integration is
//! configured (see `bootstrap/channels.rs`). `router.rs` and
//! `telegram/mod.rs` must stay free of any reference to it.

use std::sync::Arc;

use crate::ports::{ChannelReadApplicationPort, ChannelReadDecision, ExternalAccountDirectory};
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// external account directory
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ExternalReadPreprocessor
// ---------------------------------------------------------------------------

/// [`ChannelReadApplicationPort`] that gates bounded read-only external-source intents behind
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
impl ChannelReadApplicationPort for ExternalReadPreprocessor {
    async fn preprocess(&self, principal: &str, text: &str) -> anyhow::Result<ChannelReadDecision> {
        if !is_external_read_query(text) {
            return Ok(ChannelReadDecision::Passthrough);
        }
        let labels = self.accounts.active_account_labels(principal).await?;
        if labels.len() > 1 {
            return Ok(ChannelReadDecision::Reply(account_choice_prompt(&labels)));
        }
        if let Some(label) = labels.first() {
            return Ok(ChannelReadDecision::Rewrite(selected_account_context(
                label, text,
            )));
        }
        Ok(ChannelReadDecision::Passthrough)
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
