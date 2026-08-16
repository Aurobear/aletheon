//! RA-05 PR-C Runtime AgentSupervisor writer (Agent Kernel V2).
//!
//! The deployable Agent writer: the Runtime assigns the child/generation/run
//! ID, resolves the delegate through the single generic `DelegateBackendRegistry`,
//! and exposes one spawn/wait/cancel entry.  A running AgentRun is pinned to
//! its backend generation (reload never swaps a running binding). Pi concrete
//! files are NOT migrated here (that is E6-K6d). The host may still use a
//! compatibility launcher, but Runtime owns the typed run receipt and
//! lifecycle fence at this boundary.

use crate::agent_supervisor::{
    DelegateBackend, DelegateBackendId, DelegateBackendRegistry, DelegateMessage, DelegateReceipt,
    DelegateRecoveryRequest, DelegateSpawnRequest,
};
use crate::error::RuntimeError;
use crate::event::TurnTerminal;
use crate::ids::{AgentRunId, Generation, SessionId, TurnId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Runtime-owned logical child identity mint used by host adapters that still
/// expose the legacy `::contracts::AgentId` wire type. The UUID value is generated
/// here, never by a model, transport, or backend receipt.
pub fn mint_agent_run_uuid() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

/// Runtime AgentSupervisor writer over the generic registry.
pub struct RuntimeAgentSupervisor {
    registry: DelegateBackendRegistry,
    sink: Option<Arc<dyn crate::AgentEventSink>>,
    /// Run ID → pinned backend handle (reload never swaps a running binding).
    bindings: std::sync::Mutex<HashMap<AgentRunId, AgentBinding>>,
    /// Authoritative terminal fence. A late backend receipt can only observe
    /// this value; it cannot reverse it or append a second terminal.
    terminals: std::sync::Mutex<HashMap<AgentRunId, TurnTerminal>>,
    /// Serialize journal append plus terminal-map publication. Without this
    /// lock two concurrent wait/cancel/recovery paths can both pass the map
    /// check before either terminal event is durably appended.
    terminal_write_lock: tokio::sync::Mutex<()>,
    /// Starts observed from a rich host AgentControl adapter. The Runtime
    /// remains the identity and lifecycle journal authority even when the
    /// backend needs Executive-only admission fields.
    observed_starts: std::sync::Mutex<HashMap<AgentRunId, AgentStartRecord>>,
    /// Admissions recorded before the host projection is written.  Keeping
    /// this separate from `observed_starts` preserves the queued-vs-running
    /// lifecycle boundary while still allowing recovery to fence an admitted
    /// orphaned run.
    accepted: std::sync::Mutex<HashMap<AgentRunId, AgentStartRecord>>,
    /// Idempotency fence for recovery receipts. A replayed crash window may
    /// ask the host to persist the same disposition again; the Runtime stream
    /// must not grow duplicate recovery records for that exact decision.
    recoveries: std::sync::Mutex<HashMap<AgentRunId, String>>,
    /// Last durable lifecycle state for each opaque mailbox delivery. A
    /// retry of the same state is an acknowledgement; Pending may advance
    /// once to Delivered/Rejected, but a terminal delivery state cannot be
    /// rewritten.
    mailbox_deliveries:
        std::sync::Mutex<HashMap<(AgentRunId, String), crate::AgentMailboxDelivery>>,
    /// Optional host adapter for runs admitted by a rich host integration.
    /// Runtime still owns the identity/generation and terminal fence; the
    /// adapter only supplies the concrete mailbox/process operations.
    observed_backend: std::sync::RwLock<Option<Arc<dyn ObservedAgentBackend>>>,
    /// Admission gate used by maintenance/rollback. A read guard is held
    /// across backend spawn, so a drain cannot race a new child into the
    /// active set after admission has been frozen.
    admission_gate: tokio::sync::RwLock<()>,
    draining: std::sync::atomic::AtomicBool,
    next_generation: AtomicU64,
    next_stream_sequence: AtomicU64,
}

struct AgentBinding {
    backend: Arc<dyn DelegateBackend>,
    backend_run: AgentRunId,
    parent_session: SessionId,
    generation: Generation,
}

#[derive(Clone)]
struct AgentStartRecord {
    session: SessionId,
    generation: Generation,
}

/// Concrete host operations for an Agent admitted through `record_started`.
/// This is deliberately narrower than `DelegateBackend`: a host adapter may
/// not mint a child identity or select a backend generation after admission.
#[async_trait::async_trait]
pub trait ObservedAgentBackend: Send + Sync {
    async fn cancel(&self, agent_run: &AgentRunId) -> Result<(), RuntimeError>;
    async fn send(
        &self,
        agent_run: &AgentRunId,
        message: &DelegateMessage,
    ) -> Result<(), RuntimeError>;
    async fn send_with_receipt(
        &self,
        agent_run: &AgentRunId,
        message: &DelegateMessage,
    ) -> Result<crate::DelegateMessageReceipt, RuntimeError> {
        self.send(agent_run, message).await?;
        Ok(crate::DelegateMessageReceipt {
            delivery_id: message.delivery_id.clone(),
            sequence: None,
            delivered: true,
        })
    }
    async fn wait(&self, agent_run: &AgentRunId) -> Result<TurnTerminal, RuntimeError>;

    /// Resume a durable checkpoint for an observed host run after the
    /// supervisor has been recreated.  Observed runs intentionally do not
    /// have a process-local `AgentBinding` on startup; recovery must still
    /// use the composition-pinned host adapter rather than failing merely
    /// because the old in-memory binding is gone.
    async fn resume_from_checkpoint(
        &self,
        _request: &DelegateRecoveryRequest,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::UnsupportedRequest)
    }
}

/// Projection presence callback used by Runtime startup recovery to fence an
/// admitted run whose host-side projection write was interrupted.  The
/// callback is read-only; Runtime remains the writer of the recovery and
/// terminal receipts.
#[async_trait::async_trait]
pub trait AgentProjectionPresence: Send + Sync {
    async fn has_projection(&self, agent_run: &AgentRunId) -> Result<bool, RuntimeError>;
}

impl RuntimeAgentSupervisor {
    pub fn new(registry: DelegateBackendRegistry) -> Self {
        Self {
            registry,
            sink: None,
            bindings: std::sync::Mutex::new(HashMap::new()),
            terminals: std::sync::Mutex::new(HashMap::new()),
            terminal_write_lock: tokio::sync::Mutex::new(()),
            observed_starts: std::sync::Mutex::new(HashMap::new()),
            accepted: std::sync::Mutex::new(HashMap::new()),
            recoveries: std::sync::Mutex::new(HashMap::new()),
            mailbox_deliveries: std::sync::Mutex::new(HashMap::new()),
            observed_backend: std::sync::RwLock::new(None),
            admission_gate: tokio::sync::RwLock::new(()),
            draining: std::sync::atomic::AtomicBool::new(false),
            next_generation: AtomicU64::new(1),
            next_stream_sequence: AtomicU64::new(1),
        }
    }

    /// Bind the durable AgentStream sink supplied by the composition root.
    /// The supervisor remains usable in read-only unit fixtures when omitted.
    pub fn with_event_sink(mut self, sink: Arc<dyn crate::AgentEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    async fn append_event(&self, event: crate::RuntimeEvent) -> Result<(), RuntimeError> {
        let Some(sink) = &self.sink else {
            return Ok(());
        };
        let sequence = self.next_stream_sequence.fetch_add(1, Ordering::Relaxed);
        sink.append(crate::AgentStreamEvent::new(sequence, event))
            .await
    }

    /// Attach the one host adapter used for rich AgentControl runs. This is
    /// set once by the composition root after both sides of the cycle exist.
    pub fn set_observed_backend(&self, backend: Arc<dyn ObservedAgentBackend>) {
        *self.observed_backend.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(backend);
    }

    /// Record Runtime admission before the host's SQL AgentRun projection.
    /// This is the single writer-side identity receipt for a rich legacy
    /// adapter path; the projection may only follow it.
    pub async fn record_accepted(
        &self,
        session: SessionId,
        agent_run: AgentRunId,
        generation: Generation,
        backend: Option<String>,
    ) -> Result<(), RuntimeError> {
        let _admission_guard = self.admission_gate.read().await;
        if self.draining.load(Ordering::Acquire) {
            return Err(RuntimeError::Retired);
        }
        self.record_accepted_inner(session, agent_run, generation, backend)
            .await
    }

    async fn record_accepted_inner(
        &self,
        session: SessionId,
        agent_run: AgentRunId,
        generation: Generation,
        backend: Option<String>,
    ) -> Result<(), RuntimeError> {
        // Admission is a lifecycle write, not just a cache update. Serialize
        // the check, journal append, and publication so two concurrent host
        // adapters cannot both append an Accepted receipt for one run.
        let _write_guard = self.terminal_write_lock.lock().await;
        if let Some(existing) = self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&agent_run).cloned() {
            if existing.session == session && existing.generation == generation {
                return Ok(());
            }
            return Err(if existing.session != session {
                RuntimeError::WrongSession
            } else {
                RuntimeError::WrongGeneration
            });
        }
        if self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(&agent_run) {
            return Err(RuntimeError::AlreadyTerminal);
        }
        self.append_event(crate::RuntimeEvent::AgentRunAccepted {
            session: session.clone(),
            agent_run: agent_run.clone(),
            generation: generation.clone(),
            backend,
        })
        .await?;
        self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(
            agent_run,
            AgentStartRecord {
                session,
                generation,
            },
        );
        Ok(())
    }

    /// Register a backend after the composition root has constructed a host
    /// adapter. The registry itself is process-wide and rejects duplicate IDs.
    pub fn register_backend(
        &self,
        id: DelegateBackendId,
        backend: Arc<dyn DelegateBackend>,
    ) -> Result<(), RuntimeError> {
        self.registry.register(id, backend)
    }

    /// Withdraw a backend from new Runtime admission during package reload.
    /// Existing bindings retain their backend Arc and remain generation-pinned.
    pub fn unregister_backend(&self, id: &DelegateBackendId) -> bool {
        self.registry.unregister(id)
    }

    /// Register a production delegate together with the Runtime-owned
    /// capability manifest. Selection and execution therefore share one
    /// registry, while the host adapter remains responsible only for the
    /// concrete process launch.
    pub fn register_backend_with_manifest(
        &self,
        id: DelegateBackendId,
        backend: Arc<dyn DelegateBackend>,
        manifest: crate::RuntimeManifest,
    ) -> Result<(), RuntimeError> {
        self.registry.register_with_manifest(id, backend, manifest)
    }

    pub fn replace_backend_manifest(
        &self,
        id: DelegateBackendId,
        manifest: crate::RuntimeManifest,
    ) -> Result<(), RuntimeError> {
        self.registry.replace_manifest(id, manifest)
    }

    pub fn catalog(&self) -> Vec<crate::RuntimeManifest> {
        self.registry.catalog()
    }

    pub fn select(
        &self,
        request: &crate::RuntimeSelectionRequest,
    ) -> Result<(DelegateBackendId, crate::RuntimeSelectionDecision), RuntimeError> {
        self.registry.select(request)
    }

    /// Allocate a Runtime-owned public identity for a host adapter. The
    /// adapter may perform richer admission/execution, but it must use this
    /// receipt rather than minting a second Agent id.
    pub fn mint_identity(&self) -> DelegateReceipt {
        DelegateReceipt {
            agent_run: AgentRunId(format!("run-{}", mint_agent_run_uuid())),
            generation: Generation(self.next_generation.fetch_add(1, Ordering::Relaxed)),
        }
    }

    /// Identity spelling used by the legacy Fabric `AgentId(Uuid)` wire type.
    /// It is still minted by Runtime; the adapter is responsible only for the
    /// mechanical representation conversion.
    pub fn mint_legacy_agent_identity(&self) -> DelegateReceipt {
        DelegateReceipt {
            agent_run: AgentRunId(mint_agent_run_uuid().to_string()),
            generation: Generation(self.next_generation.fetch_add(1, Ordering::Relaxed)),
        }
    }

    /// Record admission from a rich host AgentControl adapter. This is
    /// idempotent for the same run/generation and fail-closed on a conflict.
    pub async fn record_started(
        &self,
        session: SessionId,
        agent_run: AgentRunId,
        generation: Generation,
        backend: Option<String>,
    ) -> Result<(), RuntimeError> {
        let _admission_guard = self.admission_gate.read().await;
        if self.draining.load(Ordering::Acquire) {
            return Err(RuntimeError::Retired);
        }
        self.record_started_inner(session, agent_run, generation, backend)
            .await
    }

    async fn record_started_inner(
        &self,
        session: SessionId,
        agent_run: AgentRunId,
        generation: Generation,
        backend: Option<String>,
    ) -> Result<(), RuntimeError> {
        // Keep the lifecycle fence around the full check/append/publication
        // sequence. Without this, concurrent admission callbacks could each
        // observe an empty map and emit duplicate Started events.
        let _write_guard = self.terminal_write_lock.lock().await;
        if let Some(existing) = self
            .observed_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&agent_run)
            .cloned()
        {
            if existing.generation == generation && existing.session == session {
                return Ok(());
            }
            return Err(if existing.session != session {
                RuntimeError::WrongSession
            } else {
                RuntimeError::WrongGeneration
            });
        }
        if self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(&agent_run) {
            return Err(RuntimeError::AlreadyTerminal);
        }
        if let Some(accepted) = self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&agent_run).cloned() {
            if accepted.session != session {
                return Err(RuntimeError::WrongSession);
            }
            if accepted.generation != generation {
                return Err(RuntimeError::WrongGeneration);
            }
        }
        self.append_event(crate::RuntimeEvent::AgentRunStarted {
            session: session.clone(),
            agent_run: agent_run.clone(),
            generation: Some(generation.clone()),
            backend: backend.clone(),
        })
        .await?;
        self.observed_starts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(
            agent_run,
            AgentStartRecord {
                session,
                generation,
            },
        );
        Ok(())
    }

    /// Record a terminal emitted by a host lifecycle adapter. Duplicate
    /// settlement is an observation of the existing terminal, not another
    /// durable event.
    pub async fn record_settled(
        &self,
        agent_run: &AgentRunId,
        terminal: TurnTerminal,
    ) -> Result<(), RuntimeError> {
        if let Some(existing) = self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run).cloned() {
            return if existing == terminal {
                Ok(())
            } else {
                Err(RuntimeError::AlreadyTerminal)
            };
        }
        let start = self
            .observed_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .cloned()
            .or_else(|| self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run).cloned())
            .ok_or(RuntimeError::AgentRunNotFound)?;
        self.settle(&start.session, agent_run, start.generation, terminal)
            .await
    }

    /// Record a successful host-mailbox delivery without reusing Session/Turn
    /// progress schemas or allowing the Runtime to forge delivery success.
    pub async fn record_message(
        &self,
        agent_run: &AgentRunId,
        kind: String,
        correlation: Option<String>,
    ) -> Result<(), RuntimeError> {
        // Mailbox observations and terminal settlement share one durable
        // publication fence.  Otherwise a terminal can be appended while a
        // stale delivery is still in flight and the replay order would no
        // longer describe the Runtime lifecycle.
        let _write_guard = self.terminal_write_lock.lock().await;
        if kind.trim().is_empty() || kind.len() > 256 {
            return Err(RuntimeError::UnsupportedRequest);
        }
        if self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(agent_run) {
            return Err(RuntimeError::AlreadyTerminal);
        }
        let session = self
            .observed_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .map(|start| start.session.clone())
            .or_else(|| {
                self.bindings
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(agent_run)
                    .map(|binding| binding.parent_session.clone())
            })
            .or_else(|| {
                self.accepted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(agent_run)
                    .map(|accepted| accepted.session.clone())
            })
            .ok_or(RuntimeError::AgentRunNotFound)?;
        self.append_event(crate::RuntimeEvent::AgentRunMessage {
            session,
            agent_run: agent_run.clone(),
            kind,
            correlation,
        })
        .await?;
        Ok(())
    }

    /// Append a Runtime-owned recovery receipt.  The host may retain richer
    /// recovery metadata in its projection, but recovery authority is not
    /// reconstructed from that SQL row.
    pub async fn record_recovery(
        &self,
        agent_run: &AgentRunId,
        decision: String,
    ) -> Result<(), RuntimeError> {
        // Recovery is an observation on the same AgentStream as terminal
        // settlement; serialize its append so a restart receipt cannot race a
        // wait/cancel terminal publication.
        let _write_guard = self.terminal_write_lock.lock().await;
        if decision.trim().is_empty() || decision.len() > 128 {
            return Err(RuntimeError::UnsupportedRequest);
        }
        // Recovery after a terminal fence is an idempotent host observation;
        // do not append a second lifecycle transition that the reducer must
        // reject as a late write.
        if self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(agent_run) {
            return Ok(());
        }
        if self
            .recoveries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .is_some_and(|existing| existing == &decision)
        {
            return Ok(());
        }
        let start = self
            .observed_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .cloned()
            .or_else(|| self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run).cloned())
            .ok_or(RuntimeError::AgentRunNotFound)?;
        self.append_event(crate::RuntimeEvent::AgentRunRecovery {
            session: start.session,
            agent_run: agent_run.clone(),
            generation: start.generation,
            decision: decision.clone(),
        })
        .await?;
        self.recoveries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(agent_run.clone(), decision);
        Ok(())
    }

    /// Append a delivery lifecycle receipt without exposing mailbox content
    /// to Runtime. Repeated identical updates are acknowledgements; a
    /// Pending receipt may advance once to Delivered/Rejected.
    pub async fn record_mailbox_delivery(
        &self,
        agent_run: &AgentRunId,
        delivery_id: String,
        kind: String,
        correlation: Option<String>,
        delivery: crate::AgentMailboxDelivery,
    ) -> Result<(), RuntimeError> {
        let _write_guard = self.terminal_write_lock.lock().await;
        if delivery_id.trim().is_empty() || kind.trim().is_empty() {
            return Err(RuntimeError::UnsupportedRequest);
        }
        if self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(agent_run) {
            return Err(RuntimeError::AlreadyTerminal);
        }
        let session = self
            .observed_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .map(|start| start.session.clone())
            .or_else(|| {
                self.accepted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(agent_run)
                    .map(|a| a.session.clone())
            })
            .ok_or(RuntimeError::AgentRunNotFound)?;
        self.append_mailbox_delivery_locked(
            agent_run,
            delivery_id,
            kind,
            correlation,
            delivery,
            session,
        )
        .await
    }

    async fn append_mailbox_delivery_locked(
        &self,
        agent_run: &AgentRunId,
        delivery_id: String,
        kind: String,
        correlation: Option<String>,
        delivery: crate::AgentMailboxDelivery,
        session: SessionId,
    ) -> Result<(), RuntimeError> {
        let key = (agent_run.clone(), delivery_id.clone());
        if let Some(existing) = self.mailbox_deliveries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&key) {
            if existing == &delivery {
                return Ok(());
            }
            if !matches!(existing, crate::AgentMailboxDelivery::Pending) {
                return Err(RuntimeError::AlreadyTerminal);
            }
        }
        self.append_event(crate::RuntimeEvent::AgentRunMailbox {
            session,
            agent_run: agent_run.clone(),
            delivery_id,
            kind,
            correlation,
            delivery: delivery.clone(),
        })
        .await?;
        self.mailbox_deliveries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, delivery);
        Ok(())
    }

    /// One spawn entry.  The Runtime assigns AgentRunId + Generation; the
    /// backend binding is resolved through the single registry and pinned.
    pub async fn spawn(
        &self,
        parent_session: &SessionId,
        parent_turn: &TurnId,
        backend: DelegateBackendId,
        profile: Option<String>,
    ) -> Result<DelegateReceipt, RuntimeError> {
        let request = DelegateSpawnRequest {
            parent_session: parent_session.clone(),
            parent_turn: Some(parent_turn.clone()),
            backend,
            profile,
            host_request: None,
            command: None,
        };
        self.spawn_request(request).await
    }

    /// Spawn a typed Runtime command. Unlike the compatibility convenience
    /// method above, this carries the host-only request through the one
    /// DelegateBackendRegistry while keeping child identity/generation owned
    /// by Runtime.
    pub async fn spawn_request(
        &self,
        request: DelegateSpawnRequest,
    ) -> Result<DelegateReceipt, RuntimeError> {
        self.spawn_request_with_mode(request, false).await
    }

    /// Host compatibility form of the typed command. Runtime still assigns
    /// the identity/generation, but uses the UUID spelling required by the
    /// legacy Fabric AgentId adapter (no `run-` transport prefix).
    pub async fn spawn_host_request(
        &self,
        request: DelegateSpawnRequest,
    ) -> Result<DelegateReceipt, RuntimeError> {
        self.spawn_request_with_mode(request, true).await
    }

    async fn spawn_request_with_mode(
        &self,
        request: DelegateSpawnRequest,
        legacy_identity: bool,
    ) -> Result<DelegateReceipt, RuntimeError> {
        // Keep this read guard until the backend has returned its receipt. A
        // maintenance drain takes the write side and therefore establishes a
        // real freeze boundary instead of relying on a racy atomic pre-check.
        let _admission_guard = self.admission_gate.read().await;
        if self.draining.load(Ordering::Acquire) {
            return Err(RuntimeError::Retired);
        }
        let parent_session = request.parent_session.clone();
        let backend = self
            .registry
            .resolve(&request.backend)
            .ok_or(RuntimeError::AgentRunNotFound)?;
        // Runtime allocates the public run identity and generation before the
        // host/backend is invoked. The backend receives this receipt and may
        // only use it; it cannot mint a replacement identity.
        let identity = if legacy_identity {
            self.mint_legacy_agent_identity()
        } else {
            self.mint_identity()
        };
        let agent_run = identity.agent_run;
        let generation = identity.generation;
        self.record_accepted_inner(
            parent_session.clone(),
            agent_run.clone(),
            generation.clone(),
            Some(request.backend.0.clone()),
        )
        .await?;
        // Publish the process-local admission before invoking the host. A
        // fast compatibility backend may emit its terminal immediately; the
        // Runtime lifecycle writer performs the check/append/publication as
        // one fenced operation.
        self.record_started_inner(
            parent_session.clone(),
            agent_run.clone(),
            generation.clone(),
            Some(request.backend.0.clone()),
        )
        .await?;
        let backend_receipt = match backend
            .spawn(
                &request,
                &DelegateReceipt {
                    agent_run: agent_run.clone(),
                    generation: generation.clone(),
                },
            )
            .await
        {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = self
                    .settle(
                        &parent_session,
                        &agent_run,
                        generation.clone(),
                        TurnTerminal::Failed {
                            message: error.to_string(),
                        },
                    )
                    .await;
                return Err(error);
            }
        };
        if backend_receipt.agent_run != agent_run || backend_receipt.generation != generation {
            let _ = backend.cancel(&backend_receipt.agent_run).await;
            let _ = self
                .settle(
                    &parent_session,
                    &agent_run,
                    generation,
                    TurnTerminal::Failed {
                        message: if backend_receipt.agent_run != agent_run {
                            "delegate backend returned a different run identity".into()
                        } else {
                            "delegate backend returned a different generation".into()
                        },
                    },
                )
                .await;
            return Err(RuntimeError::WrongGeneration);
        }
        self.bindings.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(
            agent_run.clone(),
            AgentBinding {
                backend,
                backend_run: backend_receipt.agent_run,
                parent_session: parent_session.clone(),
                generation: generation.clone(),
            },
        );
        Ok(DelegateReceipt {
            agent_run,
            generation,
        })
    }

    /// Wait for a run's authoritative terminal through its pinned backend.
    /// Never returns success before the terminal is authoritative.
    pub async fn wait(&self, agent_run: &AgentRunId) -> Result<TurnTerminal, RuntimeError> {
        // A restart recovery may have fenced an orphaned run before its
        // backend binding was rebuilt. The durable terminal is authoritative
        // and must be returned instead of treating the missing process-local
        // binding as an unknown/successful run.
        if let Some(existing) = self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run).cloned() {
            return Ok(existing);
        }
        let binding = self.bindings.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run).map(|binding| {
            (
                binding.backend.clone(),
                binding.backend_run.clone(),
                binding.parent_session.clone(),
                binding.generation.clone(),
            )
        });
        let (backend, backend_run, parent_session, generation) = match binding {
            Some(binding) => binding,
            None => {
                let observed = self
                    .observed_starts
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(agent_run)
                    .cloned()
                    .ok_or(RuntimeError::AgentRunNotFound)?;
                let host = self
                    .observed_backend
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone()
                    .ok_or(RuntimeError::AgentRunNotFound)?;
                let terminal = host.wait(agent_run).await?;
                return self
                    .settle(
                        &observed.session,
                        agent_run,
                        observed.generation,
                        terminal.clone(),
                    )
                    .await
                    .map(|()| terminal);
            }
        };
        let terminal = backend.wait(&backend_run).await?;
        self.settle(&parent_session, agent_run, generation, terminal.clone())
            .await?;
        Ok(terminal)
    }

    /// Read typed execution evidence only after the caller has observed the
    /// authoritative terminal returned by [`Self::wait`].
    pub async fn result(
        &self,
        agent_run: &AgentRunId,
    ) -> Result<crate::DelegateResult, RuntimeError> {
        if !self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(agent_run) {
            return Err(RuntimeError::NotTerminal);
        }
        let binding = self
            .bindings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .map(|binding| (binding.backend.clone(), binding.backend_run.clone()))
            .ok_or(RuntimeError::AgentRunNotFound)?;
        binding.0.result(&binding.1).await
    }

    pub async fn result_with_generation(
        &self,
        agent_run: &AgentRunId,
        expected: &Generation,
    ) -> Result<crate::DelegateResult, RuntimeError> {
        if self.generation(agent_run)? != *expected {
            return Err(RuntimeError::WrongGeneration);
        }
        self.result(agent_run).await
    }

    /// Reconcile a replayed AgentStream after a supervisor restart. Runs with
    /// a durable start but no durable terminal are fenced as Interrupted: an
    /// absent process binding is never interpreted as success. The method is
    /// intentionally fed typed Runtime events by the composition root rather
    /// than reading a second database here.
    pub async fn recover_from_events(
        &self,
        events: impl IntoIterator<Item = crate::RuntimeEvent>,
    ) -> Result<usize, RuntimeError> {
        self.replay_events(events, true).await
    }

    /// Replay the durable lifecycle state without choosing an orphan
    /// disposition.  The daemon composition uses this variant before the
    /// host projection has observed process/checkpoint state; startup
    /// reconciliation then decides Resume/Finalize/Interrupt and records that
    /// decision through the Runtime-first projection bridge.  Keeping replay
    /// separate from orphan settlement is what allows a checkpointed child to
    /// reconnect after a daemon restart instead of being interrupted merely
    /// because its process-local binding was recreated.
    pub async fn replay_from_events(
        &self,
        events: impl IntoIterator<Item = crate::RuntimeEvent>,
    ) -> Result<usize, RuntimeError> {
        self.replay_events(events, false).await
    }

    async fn replay_events(
        &self,
        events: impl IntoIterator<Item = crate::RuntimeEvent>,
        settle_orphans: bool,
    ) -> Result<usize, RuntimeError> {
        let mut started = HashMap::<AgentRunId, (SessionId, Option<Generation>)>::new();
        let mut terminal =
            HashMap::<AgentRunId, (SessionId, TurnTerminal, Option<Generation>)>::new();
        let mut max_replayed_generation = 0;
        for event in events {
            match event {
                crate::RuntimeEvent::AgentRunAccepted {
                    session,
                    agent_run,
                    generation,
                    ..
                } => {
                    max_replayed_generation = max_replayed_generation.max(generation.0);
                    let entry = (session.clone(), Some(generation.clone()));
                    if let Some(existing) = started.get(&agent_run) {
                        if existing != &entry {
                            return Err(RuntimeError::UnknownSchema);
                        }
                    }
                    self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(
                        agent_run.clone(),
                        AgentStartRecord {
                            session: session.clone(),
                            generation: generation.clone(),
                        },
                    );
                    // An accepted-but-not-started child is still an open
                    // admission. Treat it as recoverable lifecycle evidence;
                    // an absent host process will be fenced below.
                    started.insert(agent_run, entry);
                }
                crate::RuntimeEvent::AgentRunStarted {
                    session,
                    agent_run,
                    generation,
                    ..
                } => {
                    if let Some(generation) = generation.as_ref() {
                        max_replayed_generation = max_replayed_generation.max(generation.0);
                    }
                    // Older AgentStream starts omitted generation. Preserve
                    // the accepted generation when available, otherwise use
                    // the compatibility default used by the original replay.
                    let effective_generation = generation.clone().or_else(|| {
                        started
                            .get(&agent_run)
                            .and_then(|(_, generation)| generation.clone())
                    });
                    let effective_generation = effective_generation.or(Some(Generation(1)));
                    let entry = (session.clone(), effective_generation.clone());
                    if let Some(existing) = started.get(&agent_run) {
                        if existing != &entry {
                            return Err(RuntimeError::UnknownSchema);
                        }
                    }
                    self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(
                        agent_run.clone(),
                        AgentStartRecord {
                            session: session.clone(),
                            generation: effective_generation.clone().unwrap_or(Generation(1)),
                        },
                    );
                    started.insert(agent_run, entry);
                }
                crate::RuntimeEvent::AgentRunSettled {
                    session,
                    agent_run,
                    terminal: value,
                    generation,
                    ..
                } => {
                    if let Some(generation) = generation.as_ref() {
                        max_replayed_generation = max_replayed_generation.max(generation.0);
                    }
                    let entry = (session, value, generation);
                    if let Some(existing) = terminal.get(&agent_run) {
                        if existing != &entry {
                            return Err(RuntimeError::UnknownSchema);
                        }
                    }
                    terminal.insert(agent_run, entry);
                }
                crate::RuntimeEvent::AgentRunRecovery {
                    agent_run,
                    decision,
                    ..
                } => {
                    self.recoveries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(agent_run, decision);
                }
                crate::RuntimeEvent::AgentRunMailbox {
                    agent_run,
                    delivery_id,
                    delivery,
                    ..
                } => {
                    // A mailbox event after a durable terminal is late
                    // evidence and must not resurrect mailbox state during
                    // replay. Valid Pending→Delivered/Rejected updates are
                    // retained in stream order below.
                    if !terminal.contains_key(&agent_run) {
                        self.mailbox_deliveries
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .insert((agent_run, delivery_id), delivery);
                    }
                }
                _ => {}
            }
        }
        // Generation is Runtime-owned and must remain monotonic across a
        // daemon restart. Replay can restore a run at generation N before a
        // new child is admitted; without advancing this counter the fresh
        // child would reuse an old generation and weaken stale-receipt fences.
        if max_replayed_generation > 0 {
            self.next_generation
                .fetch_max(max_replayed_generation.saturating_add(1), Ordering::Relaxed);
        }
        if terminal.iter().any(|(agent_run, (session, _, _))| {
            started
                .get(agent_run)
                .is_none_or(|(started_session, _)| started_session != session)
        }) {
            return Err(RuntimeError::UnknownSchema);
        }
        let mut recovered = 0;
        let started_generations = started.clone();
        for (agent_run, (session, generation)) in started {
            let generation = generation.unwrap_or(Generation(1));
            let terminal_matches_generation =
                terminal
                    .get(&agent_run)
                    .is_some_and(|(_, _, terminal_generation)| {
                        terminal_generation
                            .as_ref()
                            .map_or(true, |terminal_generation| {
                                terminal_generation == &generation
                            })
                    });
            if terminal_matches_generation
                || self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(&agent_run)
            {
                self.observed_starts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(
                    agent_run.clone(),
                    AgentStartRecord {
                        session: session.clone(),
                        generation: generation.clone(),
                    },
                );
                continue;
            }
            self.observed_starts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(
                agent_run.clone(),
                AgentStartRecord {
                    session: session.clone(),
                    generation: generation.clone(),
                },
            );
            if !settle_orphans {
                recovered += 1;
                continue;
            }
            let value = TurnTerminal::Interrupted;
            // Persist the recovery decision separately from the terminal so
            // replay can distinguish an explicit orphan settlement from a
            // normal backend completion. This is the supported restart
            // disposition for delegates that do not offer reconnect.
            self.append_event(crate::RuntimeEvent::AgentRunRecovery {
                session: session.clone(),
                agent_run: agent_run.clone(),
                generation: generation.clone(),
                decision: "settle-orphan:interrupted".into(),
            })
            .await?;
            self.append_event(crate::RuntimeEvent::AgentRunSettled {
                session: session.clone(),
                agent_run: agent_run.clone(),
                terminal: value.clone(),
                generation: Some(generation.clone()),
            })
            .await?;
            terminal.insert(
                agent_run,
                (session.clone(), value, Some(generation.clone())),
            );
            recovered += 1;
        }
        self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).extend(
            terminal
                .into_iter()
                .filter(|(agent_run, (_, _, terminal_generation))| {
                    // A terminal from another generation is stale evidence;
                    // the loop above has already fenced this run as
                    // Interrupted when no matching receipt existed.
                    started_generations
                        .get(agent_run)
                        .is_some_and(|(_, started_generation)| {
                            terminal_generation
                                .as_ref()
                                .is_none_or(|terminal_generation| {
                                    terminal_generation
                                        == &started_generation.clone().unwrap_or(Generation(1))
                                })
                        })
                })
                .map(|(agent_run, (_, value, _))| (agent_run, value)),
        );
        Ok(recovered)
    }

    /// Replay the durable AgentStream envelope. Validation happens at the
    /// Runtime boundary, before lifecycle reconciliation can consume a
    /// payload. The logical sequence is advanced past the replayed stream so
    /// a restarted writer never emits a duplicate sequence number.
    pub async fn recover_from_stream(
        &self,
        events: impl IntoIterator<Item = crate::AgentStreamEvent>,
    ) -> Result<usize, RuntimeError> {
        self.replay_stream(events, true).await
    }

    /// Replay a durable AgentStream while deferring orphan settlement to the
    /// host startup reconciler.  This preserves checkpointed children until
    /// their process/checkpoint observation can select Resume/Finalize/Interrupt.
    pub async fn replay_from_stream(
        &self,
        events: impl IntoIterator<Item = crate::AgentStreamEvent>,
    ) -> Result<usize, RuntimeError> {
        self.replay_stream(events, false).await
    }

    async fn replay_stream(
        &self,
        events: impl IntoIterator<Item = crate::AgentStreamEvent>,
        settle_orphans: bool,
    ) -> Result<usize, RuntimeError> {
        let mut replay = Vec::new();
        let mut max_sequence = 0;
        let mut previous_sequence = 0;
        for event in events {
            event.validate().map_err(|_| RuntimeError::UnknownSchema)?;
            if event.sequence <= previous_sequence {
                return Err(RuntimeError::UnknownSchema);
            }
            previous_sequence = event.sequence;
            max_sequence = max_sequence.max(event.sequence);
            replay.push(event.into_event());
        }
        if max_sequence > 0 {
            self.next_stream_sequence
                .fetch_max(max_sequence.saturating_add(1), Ordering::Relaxed);
        }
        if settle_orphans {
            self.recover_from_events(replay).await
        } else {
            self.replay_from_events(replay).await
        }
    }

    /// Return all non-terminal Runtime children known to this writer. The
    /// union includes accepted/observed host runs and directly bound delegate
    /// runs because a restart can occur between those lifecycle receipts.
    pub fn active_runs(&self) -> Vec<AgentRunId> {
        let mut active = std::collections::HashSet::new();
        active.extend(self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).keys().cloned());
        active.extend(self.observed_starts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).keys().cloned());
        active.extend(self.bindings.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).keys().cloned());
        let terminals = self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        let mut active = active
            .into_iter()
            .filter(|agent_run| !terminals.contains_key(agent_run))
            .collect::<Vec<_>>();
        active.sort_by(|left, right| left.0.cmp(&right.0));
        active
    }

    /// Return open Runtime lifecycle identities together with their pinned
    /// generation. This includes the small window after Runtime admission and
    /// before a host backend binding is published. Startup reconciliation uses
    /// it to fence an accepted/started run whose rich host projection was lost
    /// before commit; `active_runs` alone cannot see that distinction.
    pub fn open_lifecycle_runs(&self) -> Vec<(AgentRunId, Generation)> {
        let terminals = self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut runs = HashMap::<AgentRunId, Generation>::new();
        for (agent_run, record) in self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter() {
            if !terminals.contains_key(agent_run) {
                runs.insert(agent_run.clone(), record.generation.clone());
            }
        }
        // Older/replayed callers may publish Started without a separate
        // Accepted receipt. Keep that lifecycle evidence visible to startup
        // reconciliation as well.
        for (agent_run, record) in self.observed_starts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter() {
            if !terminals.contains_key(agent_run) {
                runs.insert(agent_run.clone(), record.generation.clone());
            }
        }
        for (agent_run, binding) in self.bindings.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter() {
            if !terminals.contains_key(agent_run) {
                runs.insert(agent_run.clone(), binding.generation.clone());
            }
        }
        let mut runs = runs.into_iter().collect::<Vec<_>>();
        runs.sort_by(|left, right| left.0 .0.cmp(&right.0 .0));
        runs
    }

    /// Fence Runtime-admitted UUID runs that have no host projection.  This
    /// crash-window reconciliation belongs to the Runtime supervisor rather
    /// than the Executive facade; the host only answers the presence query.
    pub async fn reconcile_missing_projections<P>(
        &self,
        projections: &P,
    ) -> Result<usize, RuntimeError>
    where
        P: AgentProjectionPresence,
    {
        let mut interrupted = 0usize;
        for (agent_run, generation) in self.open_lifecycle_runs() {
            if uuid::Uuid::parse_str(&agent_run.0).is_err() {
                continue;
            }
            if projections.has_projection(&agent_run).await? {
                continue;
            }
            self.record_recovery(&agent_run, "host:missing-projection".into())
                .await?;
            self.record_settled(&agent_run, TurnTerminal::Interrupted)
                .await
                .map_err(|error| {
                    tracing::warn!(
                        agent_run = %agent_run.0,
                        generation = generation.0,
                        %error,
                        "Runtime orphan settlement failed"
                    );
                    error
                })?;
            interrupted = interrupted.saturating_add(1);
        }
        Ok(interrupted)
    }

    /// Return the Runtime-owned session bound to an admitted run.  Host
    /// projections may have a different durable root-agent spelling after
    /// Runtime mints an identity, but lifecycle writes must continue using
    /// the original admission session rather than reconstructing it from the
    /// projected row.
    pub fn session(&self, agent_run: &AgentRunId) -> Result<SessionId, RuntimeError> {
        if let Some(start) = self.accepted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run) {
            return Ok(start.session.clone());
        }
        if let Some(start) = self.observed_starts.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run) {
            return Ok(start.session.clone());
        }
        if let Some(binding) = self.bindings.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run) {
            return Ok(binding.parent_session.clone());
        }
        Err(RuntimeError::AgentRunNotFound)
    }

    /// Freeze new delegate admission, cancel every active child, and leave
    /// the writer retired for a launcher rollback or daemon shutdown. All
    /// children are attempted even if one backend reports an error; the first
    /// error is returned so callers cannot claim a successful drain without
    /// observing every cancellation result.
    pub async fn drain_active(&self) -> Result<Vec<AgentRunId>, RuntimeError> {
        let _admission_guard = self.admission_gate.write().await;
        self.draining.store(true, Ordering::Release);
        let active = self.active_runs();
        let mut first_error = None;
        for agent_run in &active {
            if let Err(error) = self.cancel(agent_run).await {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(active),
        }
    }

    /// Atomically perform the Runtime side of a launcher rollback: freeze
    /// admission, drain active children, replace the backend binding, and
    /// reopen admission only after the replacement is registered. On any
    /// failure the writer remains draining so a caller cannot accidentally
    /// admit work against a half-switched launcher.
    pub async fn replace_backend_after_drain(
        &self,
        id: DelegateBackendId,
        backend: Arc<dyn DelegateBackend>,
        manifest: Option<crate::RuntimeManifest>,
    ) -> Result<Vec<AgentRunId>, RuntimeError> {
        let drained = self.drain_active().await?;
        self.unregister_backend(&id);
        let result = match manifest {
            Some(manifest) => self.register_backend_with_manifest(id, backend, manifest),
            None => self.register_backend(id, backend),
        };
        result?;
        self.resume_admission()?;
        Ok(drained)
    }

    /// Re-open admission after a completed maintenance/rollback sequence.
    /// Callers must only invoke this after the old active set has been
    /// drained and the replacement backend has been registered.
    pub fn resume_admission(&self) -> Result<(), RuntimeError> {
        if !self.is_draining() || !self.active_runs().is_empty() {
            return Err(RuntimeError::MaintenanceNotDrained);
        }
        self.draining.store(false, Ordering::Release);
        Ok(())
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }

    /// Cancel a run through its pinned backend.
    pub async fn cancel(&self, agent_run: &AgentRunId) -> Result<(), RuntimeError> {
        let binding = self.bindings.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run).map(|binding| {
            (
                binding.backend.clone(),
                binding.backend_run.clone(),
                binding.parent_session.clone(),
                binding.generation.clone(),
            )
        });
        let (backend, backend_run, session, generation) = match binding {
            Some(binding) => binding,
            None => {
                let observed = self
                    .observed_starts
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(agent_run)
                    .cloned()
                    .ok_or(RuntimeError::AgentRunNotFound)?;
                let host = self
                    .observed_backend
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone()
                    .ok_or(RuntimeError::AgentRunNotFound)?;
                host.cancel(agent_run).await?;
                return self
                    .settle(
                        &observed.session,
                        agent_run,
                        observed.generation,
                        TurnTerminal::Interrupted,
                    )
                    .await;
            }
        };
        backend.cancel(&backend_run).await?;
        self.settle(&session, agent_run, generation, TurnTerminal::Interrupted)
            .await
    }

    /// Send through the backend pinned at spawn. A terminal run rejects new
    /// mailbox traffic, and an unsupported native mailbox fails closed.
    pub async fn send(
        &self,
        agent_run: &AgentRunId,
        message: DelegateMessage,
    ) -> Result<(), RuntimeError> {
        self.send_with_receipt(agent_run, message).await.map(|_| ())
    }

    /// Send through the pinned backend and preserve a bounded host delivery
    /// receipt for callers that still expose the richer Fabric message.
    pub async fn send_with_receipt(
        &self,
        agent_run: &AgentRunId,
        message: DelegateMessage,
    ) -> Result<crate::DelegateMessageReceipt, RuntimeError> {
        message.validate()?;
        if self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(agent_run) {
            return Err(RuntimeError::AlreadyTerminal);
        }
        let binding = self
            .bindings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .map(|binding| (binding.backend.clone(), binding.backend_run.clone()));
        let receipt = if let Some((backend, backend_run)) = binding {
            backend.send_with_receipt(&backend_run, &message).await?
        } else {
            let host = self
                .observed_backend
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
                .ok_or(RuntimeError::AgentRunNotFound)?;
            host.send_with_receipt(agent_run, &message).await?
        };
        self.record_send_observation(agent_run, &message, &receipt)
            .await?;
        Ok(receipt)
    }

    /// Persist the Runtime lifecycle evidence for a successful mailbox send.
    /// The host/backend owns message content and delivery mechanics; Runtime
    /// records only the bounded message and delivery observations. Both
    /// observations share one append/fence critical section so a terminal
    /// cannot be published between them.
    async fn record_send_observation(
        &self,
        agent_run: &AgentRunId,
        message: &DelegateMessage,
        receipt: &crate::DelegateMessageReceipt,
    ) -> Result<(), RuntimeError> {
        let _write_guard = self.terminal_write_lock.lock().await;
        if self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(agent_run) {
            return Err(RuntimeError::AlreadyTerminal);
        }
        if receipt
            .delivery_id
            .as_ref()
            .is_some_and(|delivery_id| delivery_id.trim().is_empty())
        {
            return Err(RuntimeError::UnsupportedRequest);
        }
        let session = self
            .observed_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .map(|start| start.session.clone())
            .or_else(|| {
                self.bindings
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(agent_run)
                    .map(|binding| binding.parent_session.clone())
            })
            .or_else(|| {
                self.accepted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(agent_run)
                    .map(|accepted| accepted.session.clone())
            })
            .ok_or(RuntimeError::AgentRunNotFound)?;
        self.append_event(crate::RuntimeEvent::AgentRunMessage {
            session: session.clone(),
            agent_run: agent_run.clone(),
            kind: message.kind.clone(),
            correlation: message.correlation.clone(),
        })
        .await?;
        if let Some(delivery_id) = receipt.delivery_id.clone() {
            self.append_mailbox_delivery_locked(
                agent_run,
                delivery_id,
                message.kind.clone(),
                message.correlation.clone(),
                if receipt.delivered {
                    crate::AgentMailboxDelivery::Delivered
                } else {
                    crate::AgentMailboxDelivery::Rejected
                },
                session,
            )
            .await?;
        }
        Ok(())
    }

    /// Generation-fenced mailbox send. A caller that cached an older
    /// AgentRun receipt cannot deliver into a newer binding after recovery or
    /// re-admission.
    pub async fn send_with_generation(
        &self,
        agent_run: &AgentRunId,
        expected: &Generation,
        message: DelegateMessage,
    ) -> Result<crate::DelegateMessageReceipt, RuntimeError> {
        if self.generation(agent_run)? != *expected {
            return Err(RuntimeError::WrongGeneration);
        }
        self.send_with_receipt(agent_run, message).await
    }

    /// Generation-fenced wait. Callers using a stale receipt cannot observe
    /// or mutate a newer Runtime generation.
    pub async fn wait_with_generation(
        &self,
        agent_run: &AgentRunId,
        expected: &Generation,
    ) -> Result<TurnTerminal, RuntimeError> {
        if self.generation(agent_run)? != *expected {
            return Err(RuntimeError::WrongGeneration);
        }
        self.wait(agent_run).await
    }

    pub async fn cancel_with_generation(
        &self,
        agent_run: &AgentRunId,
        expected: &Generation,
    ) -> Result<(), RuntimeError> {
        if self.generation(agent_run)? != *expected {
            return Err(RuntimeError::WrongGeneration);
        }
        self.cancel(agent_run).await
    }

    /// Resume through the backend pinned to this Runtime generation. This is
    /// deliberately separate from ordinary spawn: recovery cannot reselect a
    /// backend or mint a replacement identity.
    pub async fn resume_from_checkpoint(
        &self,
        agent_run: &AgentRunId,
        expected: &Generation,
        checkpoint_reference: String,
    ) -> Result<(), RuntimeError> {
        if self.generation(agent_run)? != *expected {
            return Err(RuntimeError::WrongGeneration);
        }
        let binding = self
            .bindings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .map(|binding| (binding.backend.clone(), binding.backend_run.clone()));
        let Some((backend, backend_run)) = binding else {
            let observed = self
                .observed_starts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(agent_run)
                .cloned()
                .ok_or(RuntimeError::AgentRunNotFound)?;
            let host = self
                .observed_backend
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
                .ok_or(RuntimeError::AgentRunNotFound)?;
            return host
                .resume_from_checkpoint(&DelegateRecoveryRequest {
                    agent_run: agent_run.clone(),
                    generation: observed.generation,
                    checkpoint_reference,
                })
                .await;
        };
        backend
            .resume_from_checkpoint(&DelegateRecoveryRequest {
                agent_run: backend_run.clone(),
                generation: expected.clone(),
                checkpoint_reference,
            })
            .await
    }

    async fn settle(
        &self,
        session: &SessionId,
        agent_run: &AgentRunId,
        generation: Generation,
        terminal: TurnTerminal,
    ) -> Result<(), RuntimeError> {
        let _write_guard = self.terminal_write_lock.lock().await;
        if let Some(existing) = self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run).cloned() {
            if existing == terminal {
                return Ok(());
            }
            return Err(RuntimeError::AlreadyTerminal);
        }
        self.append_event(crate::RuntimeEvent::AgentRunSettled {
            session: session.clone(),
            agent_run: agent_run.clone(),
            terminal: terminal.clone(),
            generation: Some(generation),
        })
        .await?;
        self.terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(agent_run.clone(), terminal);
        Ok(())
    }

    pub fn generation(&self, agent_run: &AgentRunId) -> Result<Generation, RuntimeError> {
        if let Some(generation) = self
            .bindings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .map(|binding| binding.generation.clone())
        {
            return Ok(generation);
        }
        self.observed_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_run)
            .map(|start| start.generation.clone())
            .or_else(|| {
                self.accepted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(agent_run)
                    .map(|accepted| accepted.generation.clone())
            })
            .ok_or(RuntimeError::AgentRunNotFound)
    }

    /// Return the durable terminal already fenced for an Agent run, if any.
    /// Host projection adapters use this to make recovery settlement
    /// idempotent without attempting a second `Started` transition after the
    /// Runtime has already fenced the orphan.
    pub fn terminal(&self, agent_run: &AgentRunId) -> Option<TurnTerminal> {
        self.terminals.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(agent_run).cloned()
    }

    pub fn registry(&self) -> &DelegateBackendRegistry {
        &self.registry
    }
}

/// A backend handle pinned to a running AgentRun.
pub struct StubBackend;

#[async_trait::async_trait]
impl DelegateBackend for StubBackend {
    async fn spawn(
        &self,
        _r: &DelegateSpawnRequest,
        identity: &DelegateReceipt,
    ) -> Result<DelegateReceipt, RuntimeError> {
        Ok(DelegateReceipt {
            agent_run: identity.agent_run.clone(),
            generation: identity.generation.clone(),
        })
    }
    async fn cancel(&self, _a: &AgentRunId) -> Result<(), RuntimeError> {
        Ok(())
    }
    async fn wait(&self, _a: &AgentRunId) -> Result<TurnTerminal, RuntimeError> {
        Ok(TurnTerminal::Completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentStreamEvent, RuntimeEvent};

    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<crate::AgentStreamEvent>>);

    #[async_trait::async_trait]
    impl crate::AgentEventSink for RecordingSink {
        async fn append(&self, event: crate::AgentStreamEvent) -> Result<(), RuntimeError> {
            self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(event);
            Ok(())
        }
    }

    struct IdentityChangingBackend;

    #[async_trait::async_trait]
    impl DelegateBackend for IdentityChangingBackend {
        async fn spawn(
            &self,
            _request: &DelegateSpawnRequest,
            identity: &DelegateReceipt,
        ) -> Result<DelegateReceipt, RuntimeError> {
            Ok(DelegateReceipt {
                agent_run: AgentRunId(format!("different-{}", identity.agent_run.0)),
                generation: identity.generation.clone(),
            })
        }

        async fn cancel(&self, _agent_run: &AgentRunId) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn wait(&self, _agent_run: &AgentRunId) -> Result<TurnTerminal, RuntimeError> {
            Ok(TurnTerminal::Completed)
        }
    }

    struct ResumeBackend {
        resumed: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait::async_trait]
    impl DelegateBackend for ResumeBackend {
        async fn spawn(
            &self,
            _request: &DelegateSpawnRequest,
            identity: &DelegateReceipt,
        ) -> Result<DelegateReceipt, RuntimeError> {
            Ok(identity.clone())
        }

        async fn cancel(&self, _agent_run: &AgentRunId) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn wait(&self, _agent_run: &AgentRunId) -> Result<TurnTerminal, RuntimeError> {
            Ok(TurnTerminal::Completed)
        }

        async fn resume_from_checkpoint(
            &self,
            request: &DelegateRecoveryRequest,
        ) -> Result<(), RuntimeError> {
            assert_eq!(request.checkpoint_reference, "checkpoint-1");
            self.resumed
                .store(true, std::sync::atomic::Ordering::Release);
            Ok(())
        }
    }

    #[tokio::test]
    async fn supervisor_spawns_and_wait_returns_authoritative_terminal() {
        let registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        let supervisor = RuntimeAgentSupervisor::new(registry);
        let receipt = supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t1".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .unwrap();
        assert!(receipt.agent_run.0.starts_with("run-"));
        assert_eq!(receipt.generation, Generation(1));
        assert_eq!(
            supervisor.session(&receipt.agent_run).unwrap(),
            SessionId("s1".into())
        );
        // Wait returns the authoritative terminal (typed, never guessed).
        let terminal = supervisor.wait(&receipt.agent_run).await.unwrap();
        assert_eq!(terminal, TurnTerminal::Completed);
    }

    #[tokio::test]
    async fn cancel_uses_the_pinned_backend() {
        let registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        let supervisor = RuntimeAgentSupervisor::new(registry);
        let receipt = supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t1".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .unwrap();
        assert!(supervisor.cancel(&receipt.agent_run).await.is_ok());
    }

    #[tokio::test]
    async fn recovery_uses_the_pinned_backend_generation() {
        let resumed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let registry = DelegateBackendRegistry::new();
        registry
            .register(
                DelegateBackendId("checkpointed".into()),
                Arc::new(ResumeBackend {
                    resumed: resumed.clone(),
                }),
            )
            .unwrap();
        let supervisor = RuntimeAgentSupervisor::new(registry);
        let receipt = supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t1".into()),
                DelegateBackendId("checkpointed".into()),
                None,
            )
            .await
            .unwrap();

        supervisor
            .resume_from_checkpoint(
                &receipt.agent_run,
                &receipt.generation,
                "checkpoint-1".into(),
            )
            .await
            .unwrap();
        assert!(resumed.load(std::sync::atomic::Ordering::Acquire));
        assert!(supervisor
            .resume_from_checkpoint(
                &receipt.agent_run,
                &Generation(receipt.generation.0 + 1),
                "checkpoint-1".into(),
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn concurrent_host_admission_is_idempotent_and_single_append() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = Arc::new(
            RuntimeAgentSupervisor::new(DelegateBackendRegistry::new())
                .with_event_sink(sink.clone()),
        );
        let identity = supervisor.mint_legacy_agent_identity();
        let session = SessionId("root".into());

        let (accepted_left, accepted_right) = tokio::join!(
            supervisor.record_accepted(
                session.clone(),
                identity.agent_run.clone(),
                identity.generation.clone(),
                Some("native".into()),
            ),
            supervisor.record_accepted(
                session.clone(),
                identity.agent_run.clone(),
                identity.generation.clone(),
                Some("native".into()),
            ),
        );
        accepted_left.unwrap();
        accepted_right.unwrap();

        let (started_left, started_right) = tokio::join!(
            supervisor.record_started(
                session.clone(),
                identity.agent_run.clone(),
                identity.generation.clone(),
                Some("native".into()),
            ),
            supervisor.record_started(
                session,
                identity.agent_run.clone(),
                identity.generation.clone(),
                Some("native".into()),
            ),
        );
        started_left.unwrap();
        started_right.unwrap();

        let events = sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.event.as_ref(),
                    RuntimeEvent::AgentRunAccepted { .. }
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.event.as_ref(),
                    RuntimeEvent::AgentRunStarted { .. }
                ))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn maintenance_drain_freezes_admission_and_fences_active_children() {
        let registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        let supervisor = RuntimeAgentSupervisor::new(registry);
        assert_eq!(
            supervisor.resume_admission(),
            Err(RuntimeError::MaintenanceNotDrained)
        );
        let receipt = supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t1".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(supervisor.active_runs(), vec![receipt.agent_run.clone()]);

        // Even if a caller flips the maintenance flag early, active children
        // keep admission closed until the drain actually removes them.
        supervisor.draining.store(true, Ordering::Release);
        assert_eq!(
            supervisor.resume_admission(),
            Err(RuntimeError::MaintenanceNotDrained)
        );
        supervisor.draining.store(false, Ordering::Release);

        let drained = supervisor.drain_active().await.unwrap();
        assert_eq!(drained, vec![receipt.agent_run.clone()]);
        assert!(supervisor.is_draining());
        assert!(supervisor.active_runs().is_empty());
        assert_eq!(
            supervisor
                .spawn(
                    &SessionId("s1".into()),
                    &TurnId("t2".into()),
                    DelegateBackendId("native".into()),
                    None,
                )
                .await
                .unwrap_err(),
            RuntimeError::Retired
        );
        let rejected_identity = supervisor.mint_legacy_agent_identity();
        assert_eq!(
            supervisor
                .record_accepted(
                    SessionId("s1".into()),
                    rejected_identity.agent_run,
                    rejected_identity.generation,
                    Some("native".into()),
                )
                .await,
            Err(RuntimeError::Retired)
        );
        supervisor.resume_admission().unwrap();
        assert!(!supervisor.is_draining());
        assert!(supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t3".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn launcher_replacement_requires_a_completed_drain() {
        let registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        let supervisor = RuntimeAgentSupervisor::new(registry);
        let old = supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t1".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .unwrap();
        let drained = supervisor
            .replace_backend_after_drain(
                DelegateBackendId("native".into()),
                Arc::new(StubBackend),
                None,
            )
            .await
            .unwrap();
        assert_eq!(drained, vec![old.agent_run]);
        assert!(!supervisor.is_draining());
        assert!(supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t2".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn backend_cannot_replace_runtime_assigned_identity() {
        let registry = DelegateBackendRegistry::new();
        registry
            .register(
                DelegateBackendId("native".into()),
                Arc::new(IdentityChangingBackend),
            )
            .unwrap();
        let supervisor = RuntimeAgentSupervisor::new(registry);
        let result = supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t1".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await;
        assert_eq!(result, Err(RuntimeError::WrongGeneration));
    }

    #[tokio::test]
    async fn lifecycle_receipts_are_journaled_and_terminal_is_fenced() {
        let registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        let sink = Arc::new(RecordingSink::default());
        let supervisor = RuntimeAgentSupervisor::new(registry).with_event_sink(sink.clone());
        let session = SessionId("s1".into());
        let receipt = supervisor
            .spawn(
                &session,
                &TurnId("t1".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            supervisor.wait(&receipt.agent_run).await.unwrap(),
            TurnTerminal::Completed
        );
        // A second authoritative wait observes the fenced terminal and does
        // not append a duplicate AgentRunSettled event.
        assert_eq!(
            supervisor.wait(&receipt.agent_run).await.unwrap(),
            TurnTerminal::Completed
        );
        let events = sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);
        assert_eq!(events[2].sequence, 3);
        assert!(matches!(
            events[0].event.as_ref(),
            RuntimeEvent::AgentRunAccepted { .. }
        ));
        assert!(matches!(
            events[1].event.as_ref(),
            RuntimeEvent::AgentRunStarted { .. }
        ));
        assert!(matches!(
            events[2].event.as_ref(),
            RuntimeEvent::AgentRunSettled { .. }
        ));
        assert!(events.iter().all(|event| event.validate().is_ok()));
    }

    #[tokio::test]
    async fn accepted_recovery_and_mailbox_receipts_are_runtime_owned() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new())
            .with_event_sink(sink.clone());
        let run = AgentRunId("legacy-agent".into());
        let generation = Generation(7);
        supervisor
            .record_accepted(
                SessionId("root".into()),
                run.clone(),
                generation,
                Some("native".into()),
            )
            .await
            .unwrap();
        assert_eq!(
            supervisor.generation(&run).unwrap(),
            Generation(7),
            "accepted Runtime identity must remain visible before host start"
        );
        supervisor
            .record_started(
                SessionId("root".into()),
                run.clone(),
                Generation(7),
                Some("native".into()),
            )
            .await
            .unwrap();
        supervisor
            .record_mailbox_delivery(
                &run,
                "delivery-1".into(),
                "prompt".into(),
                Some("corr-1".into()),
                crate::AgentMailboxDelivery::Pending,
            )
            .await
            .unwrap();
        supervisor
            .record_mailbox_delivery(
                &run,
                "delivery-1".into(),
                "prompt".into(),
                Some("corr-1".into()),
                crate::AgentMailboxDelivery::Delivered,
            )
            .await
            .unwrap();
        let before_duplicate = sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len();
        supervisor
            .record_mailbox_delivery(
                &run,
                "delivery-1".into(),
                "prompt".into(),
                Some("corr-1".into()),
                crate::AgentMailboxDelivery::Delivered,
            )
            .await
            .unwrap();
        assert_eq!(sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len(), before_duplicate);
        supervisor
            .record_recovery(&run, "Interrupt".into())
            .await
            .unwrap();
        supervisor
            .record_recovery(&run, "Interrupt".into())
            .await
            .unwrap();
        let events = sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(matches!(
            events[0].event.as_ref(),
            RuntimeEvent::AgentRunAccepted { .. }
        ));
        assert!(matches!(
            events[2].event.as_ref(),
            RuntimeEvent::AgentRunMailbox {
                delivery: crate::AgentMailboxDelivery::Pending,
                ..
            }
        ));
        assert!(matches!(
            events[4].event.as_ref(),
            RuntimeEvent::AgentRunRecovery { .. }
        ));
    }

    #[tokio::test]
    async fn agent_admission_distinguishes_session_and_generation_conflicts() {
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new());
        let run = AgentRunId("agent-conflict".into());
        supervisor
            .record_accepted(
                SessionId("s1".into()),
                run.clone(),
                Generation(3),
                Some("native".into()),
            )
            .await
            .unwrap();
        assert_eq!(
            supervisor
                .record_accepted(
                    SessionId("s2".into()),
                    run.clone(),
                    Generation(3),
                    Some("native".into()),
                )
                .await,
            Err(RuntimeError::WrongSession)
        );
        assert_eq!(
            supervisor
                .record_started(
                    SessionId("s1".into()),
                    run,
                    Generation(4),
                    Some("native".into()),
                )
                .await,
            Err(RuntimeError::WrongGeneration)
        );
    }

    #[tokio::test]
    async fn replayed_mailbox_receipt_is_idempotent_after_restart() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new())
            .with_event_sink(sink.clone());
        let run = AgentRunId("replayed-mailbox-run".into());
        let session = SessionId("root".into());
        let generation = Generation(4);
        supervisor
            .replay_from_stream([
                AgentStreamEvent::new(
                    1,
                    RuntimeEvent::AgentRunAccepted {
                        session: session.clone(),
                        agent_run: run.clone(),
                        generation: generation.clone(),
                        backend: Some("native".into()),
                    },
                ),
                AgentStreamEvent::new(
                    2,
                    RuntimeEvent::AgentRunStarted {
                        session: session.clone(),
                        agent_run: run.clone(),
                        generation: Some(generation),
                        backend: Some("native".into()),
                    },
                ),
                AgentStreamEvent::new(
                    3,
                    RuntimeEvent::AgentRunMailbox {
                        session,
                        agent_run: run.clone(),
                        delivery_id: "delivery-replayed".into(),
                        kind: "input".into(),
                        correlation: None,
                        delivery: crate::AgentMailboxDelivery::Delivered,
                    },
                ),
            ])
            .await
            .unwrap();
        supervisor
            .record_mailbox_delivery(
                &run,
                "delivery-replayed".into(),
                "input".into(),
                None,
                crate::AgentMailboxDelivery::Delivered,
            )
            .await
            .unwrap();
        assert!(sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_empty());
    }

    #[tokio::test]
    async fn replayed_mailbox_after_terminal_is_ignored() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new())
            .with_event_sink(sink.clone());
        let run = AgentRunId("late-mailbox-run".into());
        let session = SessionId("root".into());
        let generation = Generation(5);
        supervisor
            .replay_from_stream([
                AgentStreamEvent::new(
                    1,
                    RuntimeEvent::AgentRunStarted {
                        session: session.clone(),
                        agent_run: run.clone(),
                        generation: Some(generation.clone()),
                        backend: Some("native".into()),
                    },
                ),
                AgentStreamEvent::new(
                    2,
                    RuntimeEvent::AgentRunSettled {
                        session: session.clone(),
                        agent_run: run.clone(),
                        terminal: TurnTerminal::Completed,
                        generation: Some(generation),
                    },
                ),
                AgentStreamEvent::new(
                    3,
                    RuntimeEvent::AgentRunMailbox {
                        session,
                        agent_run: run.clone(),
                        delivery_id: "late-delivery".into(),
                        kind: "input".into(),
                        correlation: None,
                        delivery: crate::AgentMailboxDelivery::Delivered,
                    },
                ),
            ])
            .await
            .unwrap();
        assert_eq!(
            supervisor
                .record_mailbox_delivery(
                    &run,
                    "late-delivery".into(),
                    "input".into(),
                    None,
                    crate::AgentMailboxDelivery::Delivered,
                )
                .await,
            Err(RuntimeError::AlreadyTerminal)
        );
        assert!(sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_empty());
    }

    #[tokio::test]
    async fn restart_recovery_fences_orphaned_run_as_interrupted() {
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new());
        let run = AgentRunId("run-orphan".into());
        let recovered = supervisor
            .recover_from_events([RuntimeEvent::AgentRunStarted {
                session: SessionId("s1".into()),
                agent_run: run.clone(),
                generation: Some(Generation(1)),
                backend: Some("native".into()),
            }])
            .await
            .unwrap();
        assert_eq!(recovered, 1);
        assert_eq!(
            supervisor.wait(&run).await.unwrap(),
            TurnTerminal::Interrupted
        );
    }

    #[tokio::test]
    async fn restart_recovery_ignores_terminal_from_wrong_generation() {
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new());
        let run = AgentRunId("run-generation-fence".into());
        let recovered = supervisor
            .recover_from_events([
                RuntimeEvent::AgentRunStarted {
                    session: SessionId("s1".into()),
                    agent_run: run.clone(),
                    generation: Some(Generation(2)),
                    backend: Some("native".into()),
                },
                RuntimeEvent::AgentRunSettled {
                    session: SessionId("s1".into()),
                    agent_run: run.clone(),
                    generation: Some(Generation(1)),
                    terminal: TurnTerminal::Completed,
                },
            ])
            .await
            .unwrap();
        assert_eq!(recovered, 1);
        assert_eq!(
            supervisor.wait(&run).await.unwrap(),
            TurnTerminal::Interrupted
        );
    }

    #[tokio::test]
    async fn replay_rejects_agent_terminal_bound_to_a_different_session() {
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new());
        let run = AgentRunId("run-wrong-session".into());
        let result = supervisor
            .replay_from_events([
                RuntimeEvent::AgentRunStarted {
                    session: SessionId("s1".into()),
                    agent_run: run.clone(),
                    generation: Some(Generation(1)),
                    backend: Some("native".into()),
                },
                RuntimeEvent::AgentRunSettled {
                    session: SessionId("s2".into()),
                    agent_run: run,
                    generation: Some(Generation(1)),
                    terminal: TurnTerminal::Completed,
                },
            ])
            .await;
        assert_eq!(result, Err(RuntimeError::UnknownSchema));
    }

    #[tokio::test]
    async fn stream_recovery_validates_and_advances_sequence() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new())
            .with_event_sink(sink.clone());
        let run = AgentRunId("run-stream-orphan".into());
        let replayed = crate::AgentStreamEvent::new(
            41,
            RuntimeEvent::AgentRunStarted {
                session: SessionId("s1".into()),
                agent_run: run.clone(),
                generation: Some(Generation(1)),
                backend: Some("native".into()),
            },
        );
        assert_eq!(supervisor.recover_from_stream([replayed]).await.unwrap(), 1);
        let events = sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 42);
        assert_eq!(events[1].sequence, 43);
        assert!(events.iter().all(|event| event.validate().is_ok()));
        assert!(matches!(
            events[0].event.as_ref(),
            RuntimeEvent::AgentRunRecovery { decision, .. }
                if decision == "settle-orphan:interrupted"
        ));
        assert!(matches!(
            events[1].event.as_ref(),
            RuntimeEvent::AgentRunSettled {
                terminal: TurnTerminal::Interrupted,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn stream_replay_rejects_non_monotonic_sequences() {
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new());
        let first = AgentStreamEvent::new(
            5,
            RuntimeEvent::AgentRunStarted {
                session: SessionId("s1".into()),
                agent_run: AgentRunId("run-sequence-1".into()),
                generation: Some(Generation(1)),
                backend: Some("native".into()),
            },
        );
        let duplicate = AgentStreamEvent::new(
            5,
            RuntimeEvent::AgentRunStarted {
                session: SessionId("s1".into()),
                agent_run: AgentRunId("run-sequence-2".into()),
                generation: Some(Generation(1)),
                backend: Some("native".into()),
            },
        );
        assert_eq!(
            supervisor.replay_from_stream([first, duplicate]).await,
            Err(RuntimeError::UnknownSchema)
        );
    }

    #[tokio::test]
    async fn replay_stream_defers_orphan_fence_for_host_checkpoint_reconciliation() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new())
            .with_event_sink(sink.clone());
        let run = AgentRunId("run-checkpoint-replay".into());
        let replayed = AgentStreamEvent::new(
            7,
            RuntimeEvent::AgentRunStarted {
                session: SessionId("s1".into()),
                agent_run: run.clone(),
                generation: Some(Generation(9)),
                backend: Some("checkpointed".into()),
            },
        );

        assert_eq!(supervisor.replay_from_stream([replayed]).await.unwrap(), 1);
        assert_eq!(supervisor.generation(&run).unwrap(), Generation(9));
        assert!(supervisor.terminal(&run).is_none());
        assert_eq!(supervisor.active_runs(), vec![run.clone()]);

        // A host adapter can now observe the checkpoint and decide whether to
        // resume; replay itself must not have manufactured Interrupted. Any
        // decision receipt continues after the replayed logical sequence.
        supervisor
            .record_recovery(&run, "host:resume".into())
            .await
            .unwrap();
        let events = sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 8);
        assert!(matches!(
            events[0].event.as_ref(),
            RuntimeEvent::AgentRunRecovery { decision, .. } if decision == "host:resume"
        ));
    }

    #[tokio::test]
    async fn replay_advances_generation_before_new_admission() {
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new());
        let run = AgentRunId("run-generation-replay".into());
        supervisor
            .replay_from_stream([
                AgentStreamEvent::new(
                    1,
                    RuntimeEvent::AgentRunStarted {
                        session: SessionId("s1".into()),
                        agent_run: run.clone(),
                        generation: Some(Generation(9)),
                        backend: Some("checkpointed".into()),
                    },
                ),
                AgentStreamEvent::new(
                    2,
                    RuntimeEvent::AgentRunSettled {
                        session: SessionId("s1".into()),
                        agent_run: run,
                        terminal: TurnTerminal::Interrupted,
                        generation: Some(Generation(9)),
                    },
                ),
            ])
            .await
            .unwrap();

        let next = supervisor.mint_identity();
        assert_eq!(next.generation, Generation(10));
    }

    #[tokio::test]
    async fn open_lifecycle_runs_include_unbound_admission_and_drop_terminal_runs() {
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new());
        let run = AgentRunId("00000000-0000-0000-0000-000000000001".into());
        supervisor
            .replay_from_events([RuntimeEvent::AgentRunStarted {
                session: SessionId("s1".into()),
                agent_run: run.clone(),
                generation: Some(Generation(11)),
                backend: Some("compatibility".into()),
            }])
            .await
            .unwrap();
        assert_eq!(
            supervisor.open_lifecycle_runs(),
            vec![(run.clone(), Generation(11))]
        );

        let observed_only = AgentRunId("00000000-0000-0000-0000-000000000002".into());
        supervisor
            .record_started(
                SessionId("s1".into()),
                observed_only.clone(),
                Generation(12),
                Some("compatibility".into()),
            )
            .await
            .unwrap();
        assert_eq!(
            supervisor.open_lifecycle_runs(),
            vec![
                (run.clone(), Generation(11)),
                (observed_only.clone(), Generation(12)),
            ]
        );

        supervisor
            .record_settled(&run, TurnTerminal::Interrupted)
            .await
            .unwrap();
        supervisor
            .record_settled(&observed_only, TurnTerminal::Interrupted)
            .await
            .unwrap();
        assert!(supervisor.open_lifecycle_runs().is_empty());
    }

    #[tokio::test]
    async fn host_adapter_receipts_are_generation_fenced_and_idempotent() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new())
            .with_event_sink(sink.clone());
        let identity = supervisor.mint_legacy_agent_identity();
        let session = SessionId("root".into());
        supervisor
            .record_started(
                session,
                identity.agent_run.clone(),
                identity.generation.clone(),
                Some("native".into()),
            )
            .await
            .unwrap();
        assert_eq!(
            supervisor
                .wait_with_generation(&identity.agent_run, &identity.generation)
                .await
                .unwrap_err(),
            RuntimeError::AgentRunNotFound
        );
        assert_eq!(
            supervisor.generation(&identity.agent_run).unwrap(),
            identity.generation
        );
        assert_eq!(
            supervisor
                .record_settled(&identity.agent_run, TurnTerminal::Completed)
                .await,
            Ok(())
        );
        assert_eq!(
            supervisor
                .record_settled(&identity.agent_run, TurnTerminal::Completed)
                .await,
            Ok(())
        );
        assert_eq!(sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len(), 2);
    }

    #[tokio::test]
    async fn stale_generation_and_terminal_mailbox_are_rejected() {
        let registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        let supervisor = RuntimeAgentSupervisor::new(registry);
        let identity = supervisor
            .spawn(
                &SessionId("s1".into()),
                &TurnId("t1".into()),
                DelegateBackendId("native".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            supervisor
                .wait_with_generation(&identity.agent_run, &Generation(999))
                .await
                .unwrap_err(),
            RuntimeError::WrongGeneration
        );
        supervisor
            .cancel(&identity.agent_run)
            .await
            .expect("cancel fences terminal");
        assert_eq!(
            supervisor
                .send(
                    &identity.agent_run,
                    DelegateMessage {
                        kind: "input".into(),
                        content: "late".into(),
                        correlation: None,
                        delivery_id: None,
                    },
                )
                .await
                .unwrap_err(),
            RuntimeError::AlreadyTerminal
        );
    }

    struct ObservedStub;

    #[async_trait::async_trait]
    impl ObservedAgentBackend for ObservedStub {
        async fn cancel(&self, _agent_run: &AgentRunId) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn send(
            &self,
            _agent_run: &AgentRunId,
            _message: &DelegateMessage,
        ) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn send_with_receipt(
            &self,
            _agent_run: &AgentRunId,
            _message: &DelegateMessage,
        ) -> Result<crate::DelegateMessageReceipt, RuntimeError> {
            Ok(crate::DelegateMessageReceipt {
                delivery_id: Some("delivery-observed".into()),
                sequence: Some(1),
                delivered: true,
            })
        }

        async fn wait(&self, _agent_run: &AgentRunId) -> Result<TurnTerminal, RuntimeError> {
            Ok(TurnTerminal::Completed)
        }

        async fn resume_from_checkpoint(
            &self,
            request: &DelegateRecoveryRequest,
        ) -> Result<(), RuntimeError> {
            assert_eq!(request.checkpoint_reference, "checkpoint:observed");
            Ok(())
        }
    }

    #[tokio::test]
    async fn observed_host_adapter_owns_wait_cancel_and_mailbox_operations() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new())
            .with_event_sink(sink.clone());
        supervisor.set_observed_backend(Arc::new(ObservedStub));
        let identity = supervisor.mint_identity();
        supervisor
            .record_started(
                SessionId("s1".into()),
                identity.agent_run.clone(),
                identity.generation.clone(),
                Some("compatibility".into()),
            )
            .await
            .unwrap();
        supervisor
            .send(
                &identity.agent_run,
                DelegateMessage {
                    kind: "input".into(),
                    content: "hello".into(),
                    correlation: None,
                    delivery_id: None,
                },
            )
            .await
            .unwrap();
        assert!(sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().any(|event| matches!(
            event.event.as_ref(),
            RuntimeEvent::AgentRunMessage { kind, .. } if kind == "input"
        )));
        assert!(sink.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().any(|event| matches!(
            event.event.as_ref(),
            RuntimeEvent::AgentRunMailbox {
                delivery_id,
                delivery: crate::AgentMailboxDelivery::Delivered,
                ..
            } if delivery_id == "delivery-observed"
        )));
        assert_eq!(
            supervisor.wait(&identity.agent_run).await.unwrap(),
            TurnTerminal::Completed
        );
        // Once the host wait has settled, cancellation is fenced and cannot
        // manufacture a second terminal.
        assert_eq!(
            supervisor.cancel(&identity.agent_run).await,
            Err(RuntimeError::AlreadyTerminal)
        );
    }

    #[tokio::test]
    async fn observed_host_adapter_can_resume_after_process_local_binding_is_gone() {
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new());
        let identity = supervisor.mint_identity();
        supervisor
            .replay_from_stream([AgentStreamEvent::new(
                1,
                RuntimeEvent::AgentRunStarted {
                    session: SessionId("s1".into()),
                    agent_run: identity.agent_run.clone(),
                    generation: Some(identity.generation.clone()),
                    backend: Some("compatibility".into()),
                },
            )])
            .await
            .unwrap();
        supervisor.set_observed_backend(Arc::new(ObservedStub));

        // No DelegateBackend binding exists for this observed run. Recovery
        // must use the composition-pinned host adapter rather than report an
        // unknown run after a supervisor restart.
        supervisor
            .resume_from_checkpoint(
                &identity.agent_run,
                &identity.generation,
                "checkpoint:observed".into(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn poisoned_lock_recovers_and_writer_keeps_serving() {
        let supervisor = RuntimeAgentSupervisor::new(DelegateBackendRegistry::new());

        // Poison the terminals map by panicking while its guard is held. A
        // later lock on the same mutex returns a PoisonError; the writer must
        // recover the guard instead of unwrapping and crashing the daemon.
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = supervisor.terminals.lock().unwrap();
            panic!("intentionally poison the terminals lock");
        }));
        assert!(poisoned.is_err());

        // Read path: a poisoned lock must still serve the current (empty) state.
        assert_eq!(supervisor.terminal(&AgentRunId("never-started".into())), None);

        // Write path: admission must still proceed through the recovered lock.
        supervisor
            .record_accepted(
                SessionId("s1".into()),
                AgentRunId("post-poison".into()),
                Generation(1),
                Some("native".into()),
            )
            .await
            .unwrap();
        assert_eq!(
            supervisor.generation(&AgentRunId("post-poison".into())).unwrap(),
            Generation(1)
        );
    }
}
