//! Daemon bootstrap for optional composite supplemental memory.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::composition::config::SupplementalMemoryConfig;
use corpus::tools::mcp::manager::McpManager;
use mnemosyne::supplemental::{
    RetryPolicy, SpoolLimits, SupplementalBackendConfig, SupplementalErrorCategory,
    SupplementalMemoryBackend, SupplementalSpool,
};
use mnemosyne::{CompositeMemoryHealth, CompositeMemoryService, MemoryService};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{SupplementalDeliveryWorker, SupplementalMcpAdapter, SupplementalSchemaStatus};

pub struct SupplementalMemoryRuntime {
    pub memory_service: Arc<dyn MemoryService>,
    pub health: Arc<Mutex<CompositeMemoryHealth>>,
    pub worker_task: Option<JoinHandle<()>>,
}

pub fn backend_config(config: &SupplementalMemoryConfig) -> SupplementalBackendConfig {
    SupplementalBackendConfig {
        enabled: config.enabled,
        projection_enabled: config.projection_enabled,
        server_name: config.server_name.clone(),
        read_sources: config.read_sources.clone(),
        write_source: config.write_source.clone(),
        request_timeout_ms: config.request_timeout_ms,
        delivery_batch_size: config.delivery_batch_size,
        recall_limit: config.recall_limit,
        schema_fixture: config.schema_fixture.clone(),
        schema_version: config.schema_version.clone(),
        retry: RetryPolicy {
            initial_delay_ms: config.retry_initial_ms,
            max_delay_ms: config.retry_max_ms,
            max_attempts: config.retry_max_attempts,
            max_age_secs: config.retry_max_age_secs,
        },
        spool: mnemosyne::supplemental::SpoolPolicy {
            path: config.spool_path.clone(),
            max_items: config.spool_max_items,
            max_bytes: config.spool_max_bytes,
            legacy_outbox_dir: Some(config.legacy_outbox_dir.clone()),
        },
    }
}

pub fn build_supplemental_memory_runtime(
    local: Arc<dyn MemoryService>,
    manager: Option<Arc<McpManager>>,
    config: &SupplementalMemoryConfig,
    clock: Arc<dyn fabric::Clock>,
    daemon_cancel: &CancellationToken,
) -> SupplementalMemoryRuntime {
    build_supplemental_memory_runtime_with_retention(
        local,
        manager,
        config,
        clock,
        daemon_cancel,
        None,
    )
}

pub fn build_supplemental_memory_runtime_with_retention(
    local: Arc<dyn MemoryService>,
    manager: Option<Arc<McpManager>>,
    config: &SupplementalMemoryConfig,
    clock: Arc<dyn fabric::Clock>,
    daemon_cancel: &CancellationToken,
    retention: Option<Arc<mnemosyne::RetentionRepository>>,
) -> SupplementalMemoryRuntime {
    if !config.enabled {
        return local_runtime(local, clock, false, None);
    }
    if !valid_runtime_config(config) {
        tracing::warn!("Supplemental memory configuration invalid; using local memory only");
        return local_runtime(local, clock, true, Some(SupplementalErrorCategory::Schema));
    }
    let Some(manager) = manager else {
        return local_runtime(
            local,
            clock,
            true,
            Some(SupplementalErrorCategory::Transport),
        );
    };
    let adapter = Arc::new(SupplementalMcpAdapter::new(
        manager,
        config.server_name.clone(),
        Duration::from_millis(config.request_timeout_ms),
    ));
    if adapter.health().schema != SupplementalSchemaStatus::Valid {
        return local_runtime(local, clock, true, Some(SupplementalErrorCategory::Schema));
    }
    let spool = match SupplementalSpool::open(
        &config.spool_path,
        SpoolLimits {
            max_items: config.spool_max_items,
            max_bytes: config.spool_max_bytes,
        },
    ) {
        Ok(spool) => Arc::new(spool),
        Err(error) => {
            tracing::warn!(error = %error, "Supplemental memory spool unavailable; using local memory only");
            return local_runtime(local, clock, true, Some(SupplementalErrorCategory::Spool));
        }
    };
    if config.projection_enabled {
        let now_ms = clock.wall_now().0.max(0);
        if let Err(error) =
            spool.migrate_legacy_outbox(Path::new(&config.legacy_outbox_dir), 1_000, now_ms)
        {
            tracing::warn!(error = %error, "Supplemental memory legacy outbox migration failed; using local memory only");
            return local_runtime(local, clock, true, Some(SupplementalErrorCategory::Spool));
        }
    }
    adapter.set_queue_depth(spool.queue_depth().unwrap_or_default());
    let backend_config = backend_config(config);
    let worker = if config.projection_enabled {
        match SupplementalDeliveryWorker::new(
            spool.clone(),
            adapter.clone(),
            backend_config.retry.clone(),
            format!("daemon-{}", std::process::id()),
            config.delivery_batch_size,
            30_000,
        ) {
            Ok(worker) => Some(match &retention {
                Some(repository) => worker.with_retention_repository(repository.clone()),
                None => worker,
            }),
            Err(error) => {
                tracing::warn!(error = %error, "Supplemental memory worker configuration invalid; using local memory only");
                return local_runtime(local, clock, true, Some(SupplementalErrorCategory::Spool));
            }
        }
    } else {
        None
    };
    let backend = Arc::new(SupplementalMemoryBackend::new(
        spool.clone(),
        adapter.clone(),
        backend_config.clone(),
    ));
    let composite = CompositeMemoryService::new(
        local,
        Some(backend),
        clock.clone(),
        Duration::from_millis(500),
        Duration::from_millis(config.request_timeout_ms),
    );
    let health = composite.health_handle();
    let memory_service: Arc<dyn MemoryService> = Arc::new(composite);
    let worker_task = worker.map(|worker| {
        let cancel = daemon_cancel.child_token();
        tokio::spawn(async move {
            worker.run(clock, Duration::from_secs(1), cancel).await;
        })
    });
    SupplementalMemoryRuntime {
        memory_service,
        health,
        worker_task,
    }
}

fn valid_runtime_config(config: &SupplementalMemoryConfig) -> bool {
    !config.server_name.trim().is_empty()
        && !config.write_source.trim().is_empty()
        && !config.read_sources.is_empty()
        && config
            .read_sources
            .iter()
            .all(|source| !source.trim().is_empty())
        && config.request_timeout_ms > 0
        && (1..=100).contains(&config.recall_limit)
        && (!config.projection_enabled || config.delivery_batch_size > 0)
        && config.spool_max_items > 0
        && config.spool_max_bytes > 0
        && config.retry_initial_ms > 0
        && config.retry_initial_ms <= config.retry_max_ms
        && config.retry_max_attempts > 0
        && config.retry_max_age_secs > 0
        && config.schema_version == mnemosyne::supplemental::config::PINNED_RELEASE
}

fn local_runtime(
    local: Arc<dyn MemoryService>,
    clock: Arc<dyn fabric::Clock>,
    configured: bool,
    category: Option<SupplementalErrorCategory>,
) -> SupplementalMemoryRuntime {
    let composite = CompositeMemoryService::local_only(local, clock);
    let health = composite.health_handle();
    {
        let mut value = health.lock().expect("composite health mutex poisoned");
        value.supplemental_enabled = configured;
        value.degraded = category.is_some();
        value.error_category = category;
    }
    SupplementalMemoryRuntime {
        memory_service: Arc::new(composite),
        health,
        worker_task: None,
    }
}
