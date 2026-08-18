//! Runtime-owned session context cache used by the binary composition root.

pub use runtime::ContextWorkingSet;

use runtime::ContextCompactorFactory;
use std::{path::Path, sync::Arc};

pub async fn compose_context_working_set(
    data_dir: &Path,
    session_id: String,
    max_tokens: usize,
    compaction_threshold_percent: usize,
    clock: Arc<dyn ::contracts::Clock>,
) -> anyhow::Result<ContextWorkingSet> {
    let factory = mnemosyne::context_compactor::MnemosyneContextCompactorFactory;
    ContextWorkingSet::new(
        data_dir,
        session_id,
        clock,
        factory.create(max_tokens, compaction_threshold_percent),
    )
    .await
}
