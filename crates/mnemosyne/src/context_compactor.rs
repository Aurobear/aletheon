//! Runtime context-compaction adapter owned alongside Mnemosyne's compressor.

use ::contracts::{LlmProvider, Message};
use runtime::{ContextCompactionReceipt, ContextCompactor, ContextCompactorFactory};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Clone, Copy, Default)]
pub struct MnemosyneContextCompactorFactory;

impl ContextCompactorFactory for MnemosyneContextCompactorFactory {
    fn create(&self, max_tokens: usize, threshold_percent: usize) -> Box<dyn ContextCompactor> {
        Box::new(MnemosyneContextCompactor {
            inner: crate::runtime::AdvancedCompressor::new(
                (max_tokens as f64 * 0.25) as usize,
                4_000,
                max_tokens,
            )
            .with_threshold_fraction(threshold_percent as f64 / 100.0),
            lineage: Vec::new(),
        })
    }
}

struct MnemosyneContextCompactor {
    inner: crate::runtime::AdvancedCompressor,
    lineage: Vec<ContextCompactionReceipt>,
}

impl MnemosyneContextCompactor {
    fn refresh_lineage(&mut self) {
        self.lineage = self
            .inner
            .lineage()
            .iter()
            .map(|entry| ContextCompactionReceipt {
                run_id: entry.run_id,
                strategy: entry.strategy,
                tokens_before: entry.tokens_before,
                tokens_after: entry.tokens_after,
                forced: entry.forced,
                applied: entry.applied,
                failure: entry.failure.clone(),
            })
            .collect();
    }
}

impl ContextCompactor for MnemosyneContextCompactor {
    fn lineage(&self) -> &[ContextCompactionReceipt] {
        &self.lineage
    }

    fn inherit_lineage(&mut self, lineage: &[ContextCompactionReceipt]) {
        let concrete: Vec<_> = lineage
            .iter()
            .map(|entry| crate::runtime::CompactionLineage {
                run_id: entry.run_id,
                strategy: entry.strategy,
                tokens_before: entry.tokens_before,
                tokens_after: entry.tokens_after,
                forced: entry.forced,
                applied: entry.applied,
                failure: entry.failure.clone(),
            })
            .collect();
        self.inner.inherit_lineage(&concrete);
        self.lineage = lineage.to_vec();
    }

    fn can_compact(&self, messages: &[Message]) -> bool {
        self.inner.can_compact(messages)
    }
    fn should_compact(&self, messages: &[Message]) -> bool {
        self.inner.should_compact(messages)
    }
    fn exceeds_threshold(&self, messages: &[Message], fraction: f64) -> bool {
        self.inner.exceeds_threshold(messages, fraction)
    }
    fn configure_budget(&mut self, history_budget: usize, aggressive: bool) {
        self.inner.configure_budget(history_budget, aggressive);
    }
    fn maybe_compact<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async move {
            let result = self.inner.maybe_compact(messages, llm).await;
            self.refresh_lineage();
            result
        })
    }
    fn force_compact<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
        llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async move {
            let result = self.inner.force_compact(messages, llm).await;
            self.refresh_lineage();
            result
        })
    }
}
