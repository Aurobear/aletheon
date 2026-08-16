//! Loop-independent Agent inbox and public control vocabulary.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use contracts::{Clock, Message};

use super::session_log::{
    HarnessInboxMessage, HarnessSessionEventKind, HarnessSessionId, HarnessSessionLog,
    HarnessSessionLogError, InboxSpliceOutcome, InboxTarget, UserMessageSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Running,
    Disposed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSendTarget {
    NextTurn,
    NextStep,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentSendRequest {
    pub id: String,
    pub message: Message,
    pub target: AgentSendTarget,
    pub wakeup: bool,
}

impl AgentSendRequest {
    pub fn followup(id: impl Into<String>, message: Message) -> Self {
        Self {
            id: id.into(),
            message,
            target: AgentSendTarget::NextTurn,
            wakeup: true,
        }
    }

    pub fn steer(id: impl Into<String>, message: Message) -> Self {
        Self {
            id: id.into(),
            message,
            target: AgentSendTarget::NextStep,
            wakeup: true,
        }
    }

    pub fn inject(id: impl Into<String>, message: Message) -> Self {
        Self {
            id: id.into(),
            message,
            target: AgentSendTarget::NextStep,
            wakeup: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentInboxClaim {
    pub messages: Vec<HarnessInboxMessage>,
    pub wake_requested: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AgentInboxError {
    #[error("message id must not be empty")]
    EmptyMessageId,
    #[error("message id already exists in the inbox: {0}")]
    DuplicateMessageId(String),
    #[error("inbox message must use the user role")]
    InvalidMessageRole,
    #[error(transparent)]
    Session(#[from] HarnessSessionLogError),
    #[error("agent inbox lock is poisoned")]
    Poisoned,
}

#[derive(Default)]
struct InboxState {
    next_turn: VecDeque<HarnessInboxMessage>,
    next_step: VecDeque<HarnessInboxMessage>,
    ids: BTreeSet<String>,
    wake_requested: bool,
}

/// Inbox with two typed queues and one durable splice vocabulary.
///
/// Every mutation appends its normalized event before changing the live
/// projection. A failed append therefore leaves the queue untouched.
pub struct AgentInbox {
    session: Arc<Mutex<HarnessSessionLog>>,
    clock: Arc<dyn Clock>,
    state: Mutex<InboxState>,
}

impl AgentInbox {
    pub fn new(session: Arc<Mutex<HarnessSessionLog>>, clock: Arc<dyn Clock>) -> Self {
        Self {
            session,
            clock,
            state: Mutex::new(InboxState::default()),
        }
    }

    pub fn send(&self, request: AgentSendRequest) -> Result<bool, AgentInboxError> {
        if request.id.trim().is_empty() {
            return Err(AgentInboxError::EmptyMessageId);
        }
        if request.message.role != contracts::Role::User {
            return Err(AgentInboxError::InvalidMessageRole);
        }
        let mut state = self.state.lock().map_err(|_| AgentInboxError::Poisoned)?;
        if state.ids.contains(&request.id) {
            return Err(AgentInboxError::DuplicateMessageId(request.id));
        }
        let message = HarnessInboxMessage {
            id: request.id,
            message: request.message,
            source: match request.target {
                AgentSendTarget::NextTurn => UserMessageSource::Human,
                AgentSendTarget::NextStep if request.wakeup => UserMessageSource::Steering,
                AgentSendTarget::NextStep => UserMessageSource::Injected,
            },
        };
        let (target, start) = match request.target {
            AgentSendTarget::NextTurn => (InboxTarget::NextTurn, state.next_turn.len()),
            AgentSendTarget::NextStep => (InboxTarget::NextStep, state.next_step.len()),
        };
        self.session
            .lock()
            .map_err(|_| AgentInboxError::Poisoned)?
            .append(
                self.clock.wall_now().0,
                HarnessSessionEventKind::InboxSpliced {
                    target,
                    start,
                    removed_count: 0,
                    inserted: vec![message.clone()],
                    outcome: None,
                },
                None,
                vec![],
            )?;
        state.ids.insert(message.id.clone());
        match request.target {
            AgentSendTarget::NextTurn => state.next_turn.push_back(message),
            AgentSendTarget::NextStep => state.next_step.push_back(message),
        }
        state.wake_requested |= request.wakeup;
        Ok(request.wakeup)
    }

    /// Claim every next-step message plus at most one FIFO next-turn prompt.
    pub fn claim_for_turn(&self) -> Result<AgentInboxClaim, AgentInboxError> {
        let mut state = self.state.lock().map_err(|_| AgentInboxError::Poisoned)?;
        let mut claimed = Vec::new();

        if !state.next_step.is_empty() {
            let count = state.next_step.len();
            self.append_claim(InboxTarget::NextStep, count)?;
            claimed.extend(state.next_step.drain(..));
        }
        if !state.next_turn.is_empty() {
            self.append_claim(InboxTarget::NextTurn, 1)?;
            if let Some(message) = state.next_turn.pop_front() {
                claimed.push(message);
            }
        }
        for message in &claimed {
            state.ids.remove(&message.id);
        }
        let wake_requested = std::mem::take(&mut state.wake_requested);
        Ok(AgentInboxClaim {
            messages: claimed,
            wake_requested,
        })
    }

    /// Claim steering/injected messages between steps without consuming the
    /// next queued turn prompt.
    pub fn claim_for_step(&self) -> Result<AgentInboxClaim, AgentInboxError> {
        let mut state = self.state.lock().map_err(|_| AgentInboxError::Poisoned)?;
        let count = state.next_step.len();
        let messages = if count == 0 {
            Vec::new()
        } else {
            self.append_claim(InboxTarget::NextStep, count)?;
            state.next_step.drain(..).collect::<Vec<_>>()
        };
        for message in &messages {
            state.ids.remove(&message.id);
        }
        let wake_requested = std::mem::take(&mut state.wake_requested);
        Ok(AgentInboxClaim {
            messages,
            wake_requested,
        })
    }

    pub fn discard_all(&self) -> Result<Vec<HarnessInboxMessage>, AgentInboxError> {
        let mut state = self.state.lock().map_err(|_| AgentInboxError::Poisoned)?;
        let mut discarded = Vec::new();
        if !state.next_step.is_empty() {
            let count = state.next_step.len();
            self.append_discard(InboxTarget::NextStep, count)?;
            discarded.extend(state.next_step.drain(..));
        }
        if !state.next_turn.is_empty() {
            let count = state.next_turn.len();
            self.append_discard(InboxTarget::NextTurn, count)?;
            discarded.extend(state.next_turn.drain(..));
        }
        state.ids.clear();
        state.wake_requested = false;
        Ok(discarded)
    }

    pub fn is_empty(&self) -> Result<bool, AgentInboxError> {
        let state = self.state.lock().map_err(|_| AgentInboxError::Poisoned)?;
        Ok(state.next_step.is_empty() && state.next_turn.is_empty())
    }

    fn append_claim(&self, target: InboxTarget, count: usize) -> Result<(), AgentInboxError> {
        self.append_removal(target, count, InboxSpliceOutcome::Claimed)
    }

    fn append_discard(&self, target: InboxTarget, count: usize) -> Result<(), AgentInboxError> {
        self.append_removal(target, count, InboxSpliceOutcome::Cancelled)
    }

    fn append_removal(
        &self,
        target: InboxTarget,
        count: usize,
        outcome: InboxSpliceOutcome,
    ) -> Result<(), AgentInboxError> {
        self.session
            .lock()
            .map_err(|_| AgentInboxError::Poisoned)?
            .append(
                self.clock.wall_now().0,
                HarnessSessionEventKind::InboxSpliced {
                    target,
                    start: 0,
                    removed_count: count,
                    inserted: Vec::new(),
                    outcome: Some(outcome),
                },
                None,
                vec![],
            )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCancelCause {
    User,
    Parent,
    Timeout,
    Disposed,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentControlError {
    #[error("agent is disposed")]
    Disposed,
    #[error("agent control failed: {0}")]
    Runtime(String),
}

/// Loop-independent live Agent interface. Consumers never name the concrete
/// loop driver and a bare registry entry has no teardown capability.
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> &HarnessSessionId;
    fn session_id(&self) -> &HarnessSessionId;
    fn status(&self) -> AgentStatus;
    fn send(&self, request: AgentSendRequest) -> Result<bool, AgentControlError>;
    async fn cancel(&self, cause: AgentCancelCause) -> Result<(), AgentControlError>;
    async fn when_idle(&self) -> Result<(), AgentControlError>;
}

/// Private lifecycle capability held by the Agent factory and consumer handle.
#[async_trait]
pub trait AgentTeardown: Send + Sync {
    async fn stop_and_quiesce(&self) -> Result<(), AgentControlError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentRegistryError {
    #[error("agent id must equal its session id")]
    IdentityMismatch,
    #[error("agent id is already live: {0}")]
    Collision(String),
    #[error("agent registration is stale or detached")]
    StaleRegistration,
    #[error("agent has already been announced")]
    AlreadyAnnounced,
    #[error("agent registry lock is poisoned")]
    Poisoned,
}

pub trait AgentRegistryObserver: Send + Sync {
    fn created(&self, agent: Arc<dyn Agent>);
    fn disposed(&self, id: &HarnessSessionId);
}

struct RegistryEntry {
    token: u64,
    agent: Arc<dyn Agent>,
    owner: Option<HarnessSessionId>,
    announced: bool,
    announcing: bool,
    detach_requested: bool,
}

struct AgentRegistryInner {
    next_token: AtomicU64,
    entries: Mutex<BTreeMap<String, RegistryEntry>>,
    observers: Mutex<Vec<Arc<dyn AgentRegistryObserver>>>,
}

/// Registry of already-constructed live Agents.
///
/// `enter` performs the authoritative collision check without publishing.
/// `announce` is a separate edge so setup can finish before observers see the
/// Agent. Detach is token-bound, preventing a stale registration from removing
/// a later replacement with the same stable id.
#[derive(Clone)]
pub struct AgentRegistry {
    inner: Arc<AgentRegistryInner>,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AgentRegistryInner {
                next_token: AtomicU64::new(1),
                entries: Mutex::new(BTreeMap::new()),
                observers: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn observe(
        &self,
        observer: Arc<dyn AgentRegistryObserver>,
    ) -> Result<(), AgentRegistryError> {
        self.inner
            .observers
            .lock()
            .map_err(|_| AgentRegistryError::Poisoned)?
            .push(observer);
        Ok(())
    }

    pub fn enter(
        &self,
        agent: Arc<dyn Agent>,
        owner: Option<&HarnessSessionId>,
    ) -> Result<AgentRegistration, AgentRegistryError> {
        if agent.id() != agent.session_id() {
            return Err(AgentRegistryError::IdentityMismatch);
        }
        let id = agent.id().clone();
        if id.0.trim().is_empty() {
            return Err(AgentRegistryError::IdentityMismatch);
        }
        let token = self.inner.next_token.fetch_add(1, Ordering::Relaxed);
        let mut entries = self
            .inner
            .entries
            .lock()
            .map_err(|_| AgentRegistryError::Poisoned)?;
        if entries.contains_key(&id.0) {
            return Err(AgentRegistryError::Collision(id.0));
        }
        entries.insert(
            id.0.clone(),
            RegistryEntry {
                token,
                agent,
                owner: owner.cloned(),
                announced: false,
                announcing: false,
                detach_requested: false,
            },
        );
        Ok(AgentRegistration {
            registry: Arc::downgrade(&self.inner),
            id,
            token,
        })
    }

    pub fn get(&self, id: &HarnessSessionId) -> Result<Option<Arc<dyn Agent>>, AgentRegistryError> {
        Ok(self
            .inner
            .entries
            .lock()
            .map_err(|_| AgentRegistryError::Poisoned)?
            .get(&id.0)
            .map(|entry| entry.agent.clone()))
    }

    pub fn list(&self) -> Result<Vec<Arc<dyn Agent>>, AgentRegistryError> {
        Ok(self
            .inner
            .entries
            .lock()
            .map_err(|_| AgentRegistryError::Poisoned)?
            .values()
            .map(|entry| entry.agent.clone())
            .collect())
    }

    pub fn roots(&self) -> Result<Vec<Arc<dyn Agent>>, AgentRegistryError> {
        Ok(self
            .inner
            .entries
            .lock()
            .map_err(|_| AgentRegistryError::Poisoned)?
            .values()
            .filter(|entry| entry.owner.is_none())
            .map(|entry| entry.agent.clone())
            .collect())
    }

    pub fn is_owned_by(
        &self,
        id: &HarnessSessionId,
        owner: &HarnessSessionId,
    ) -> Result<bool, AgentRegistryError> {
        Ok(self
            .inner
            .entries
            .lock()
            .map_err(|_| AgentRegistryError::Poisoned)?
            .get(&id.0)
            .is_some_and(|entry| entry.owner.as_ref() == Some(owner)))
    }
}

#[derive(Clone)]
pub struct AgentRegistration {
    registry: Weak<AgentRegistryInner>,
    id: HarnessSessionId,
    token: u64,
}

impl AgentRegistration {
    pub fn id(&self) -> &HarnessSessionId {
        &self.id
    }

    pub fn announce(&self) -> Result<(), AgentRegistryError> {
        let registry = self
            .registry
            .upgrade()
            .ok_or(AgentRegistryError::StaleRegistration)?;
        let agent = {
            let mut entries = registry
                .entries
                .lock()
                .map_err(|_| AgentRegistryError::Poisoned)?;
            let entry = entries
                .get_mut(&self.id.0)
                .filter(|entry| entry.token == self.token)
                .ok_or(AgentRegistryError::StaleRegistration)?;
            if entry.announced || entry.announcing {
                return Err(AgentRegistryError::AlreadyAnnounced);
            }
            entry.announced = true;
            entry.announcing = true;
            entry.agent.clone()
        };
        let observers = registry
            .observers
            .lock()
            .map_err(|_| AgentRegistryError::Poisoned)?
            .clone();
        for observer in observers {
            let agent = agent.clone();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                observer.created(agent);
            }));
        }

        let notify_disposed = {
            let mut entries = registry
                .entries
                .lock()
                .map_err(|_| AgentRegistryError::Poisoned)?;
            let Some(entry) = entries
                .get_mut(&self.id.0)
                .filter(|entry| entry.token == self.token)
            else {
                return Ok(());
            };
            entry.announcing = false;
            if entry.detach_requested {
                entries.remove(&self.id.0);
                true
            } else {
                false
            }
        };
        if notify_disposed {
            notify_disposed_observers(&registry, &self.id)?;
        }
        Ok(())
    }

    /// Detach the exact entry. Returns `true` only when this call removed it.
    pub fn detach(&self) -> Result<bool, AgentRegistryError> {
        let registry = self
            .registry
            .upgrade()
            .ok_or(AgentRegistryError::StaleRegistration)?;
        let announced = {
            let mut entries = registry
                .entries
                .lock()
                .map_err(|_| AgentRegistryError::Poisoned)?;
            let Some(entry) = entries
                .get_mut(&self.id.0)
                .filter(|entry| entry.token == self.token)
            else {
                return Ok(false);
            };
            if entry.announcing {
                entry.detach_requested = true;
                return Ok(false);
            }
            let announced = entry.announced;
            entries.remove(&self.id.0);
            announced
        };
        if announced {
            notify_disposed_observers(&registry, &self.id)?;
        }
        Ok(true)
    }
}

fn notify_disposed_observers(
    registry: &AgentRegistryInner,
    id: &HarnessSessionId,
) -> Result<(), AgentRegistryError> {
    let observers = registry
        .observers
        .lock()
        .map_err(|_| AgentRegistryError::Poisoned)?
        .clone();
    for observer in observers {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            observer.disposed(id);
        }));
    }
    Ok(())
}

/// Consumer-owned teardown capability paired with a bare live Agent.
pub struct AgentHandle {
    agent: Arc<dyn Agent>,
    teardown: Arc<dyn AgentTeardown>,
    registration: AgentRegistration,
    disposal: tokio::sync::OnceCell<Result<(), AgentControlError>>,
}

impl AgentHandle {
    pub fn new(
        agent: Arc<dyn Agent>,
        teardown: Arc<dyn AgentTeardown>,
        registration: AgentRegistration,
    ) -> Self {
        Self {
            agent,
            teardown,
            registration,
            disposal: tokio::sync::OnceCell::new(),
        }
    }

    pub fn agent(&self) -> Arc<dyn Agent> {
        self.agent.clone()
    }

    /// Stop, await quiescence, then detach. Concurrent calls share one result.
    pub async fn dispose(&self) -> Result<(), AgentControlError> {
        self.disposal
            .get_or_init(|| async {
                self.teardown.stop_and_quiesce().await?;
                self.registration
                    .detach()
                    .map_err(|error| AgentControlError::Runtime(error.to_string()))?;
                Ok(())
            })
            .await
            .clone()
    }
}
