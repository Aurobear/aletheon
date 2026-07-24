use async_trait::async_trait;
use executive::application::memory_consolidation_worker::MemoryConsolidationWorker;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio_util::sync::CancellationToken;
struct Memory {
    calls: AtomicUsize,
    promotions: AtomicUsize,
}
#[async_trait]
impl mnemosyne::MemoryService for Memory {
    async fn record(&self, _: mnemosyne::ExperienceEvent) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recall(&self, _: mnemosyne::RecallRequest) -> anyhow::Result<mnemosyne::RecallSet> {
        Ok(Default::default())
    }
    async fn consolidate(&self, scope: mnemosyne::MemoryScope) -> anyhow::Result<()> {
        assert_eq!(scope, mnemosyne::MemoryScope::Global);
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            anyhow::bail!("transient")
        }
        Ok(())
    }
    async fn forget(&self, _: mnemosyne::ForgetPolicy) -> anyhow::Result<mnemosyne::ForgetReceipt> {
        Ok(mnemosyne::ForgetReceipt::default())
    }
    async fn promote_facts(&self, min_confidence: f64, max_count: usize) -> anyhow::Result<usize> {
        assert_eq!(min_confidence, 0.7);
        assert_eq!(max_count, 20);
        self.promotions.fetch_add(1, Ordering::SeqCst);
        Ok(1)
    }
}
#[tokio::test]
async fn worker_retries_and_stops_on_cancellation() {
    let memory = Arc::new(Memory {
        calls: AtomicUsize::new(0),
        promotions: AtomicUsize::new(0),
    });
    let cancel = CancellationToken::new();
    let task = tokio::spawn(
        MemoryConsolidationWorker::new(memory.clone())
            .with_interval(std::time::Duration::from_millis(1))
            .run(cancel.clone()),
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while memory.calls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    task.await.unwrap();
    assert!(memory.calls.load(Ordering::SeqCst) >= 2)
}

#[tokio::test]
async fn worker_promotes_after_successful_consolidation() {
    let memory = Arc::new(Memory {
        calls: AtomicUsize::new(1),
        promotions: AtomicUsize::new(0),
    });
    let cancel = CancellationToken::new();
    let task = tokio::spawn(
        MemoryConsolidationWorker::new(memory.clone())
            .with_interval(std::time::Duration::from_millis(1))
            .with_promotion(0.7, 20)
            .run(cancel.clone()),
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while memory.promotions.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    task.await.unwrap();
}
