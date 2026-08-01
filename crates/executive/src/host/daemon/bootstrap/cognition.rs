//! Typed construction unit for metacognition and episodic memory.

use std::path::Path;
use std::sync::Arc;

use cognit::core::reflector::Reflector;
use fabric::{Clock, Subsystem, SubsystemContext, Version};
use metacog::DefaultMetaRuntime;
use mnemosyne::runtime::EpisodicMemory;
use tokio::sync::Mutex;

pub(super) struct CognitionComposition {
    pub(super) metacog: Arc<dyn metacog::MetacogService>,
    pub(super) reflector: Reflector,
    pub(super) episodic_memory: Arc<Mutex<EpisodicMemory>>,
}

pub(super) async fn compose(
    data_dir: &Path,
    clock: Arc<dyn Clock>,
) -> anyhow::Result<CognitionComposition> {
    let meta_runtime = Arc::new(
        DefaultMetaRuntime::new(Version::new(0, 1, 0), clock.clone())
            .with_genome_path(data_dir.join("genome.yaml"))
            .with_work_dir(data_dir.join("metacog-sandbox"), clock.clone())
            .with_lineage_path(data_dir.join("lineage").join("genome.jsonl"), clock.clone())?,
    );
    let metacog = Arc::new(metacog::DefaultMetacogService::with_state_path(
        meta_runtime,
        clock.clone(),
        data_dir.join("metacog-mutations.json"),
    )?);
    let reflector = Reflector::new(clock.clone());
    let mut episodic_memory = EpisodicMemory::new(data_dir.join("episodic.db"), clock);
    episodic_memory
        .init(&SubsystemContext {
            name: "episodic_memory".into(),
            working_dir: data_dir.to_path_buf(),
            config: serde_json::Value::Null,
            bus: None,
        })
        .await?;

    Ok(CognitionComposition {
        metacog,
        reflector,
        episodic_memory: Arc::new(Mutex::new(episodic_memory)),
    })
}
