use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cognit::harness::agent::{
    Agent, AgentCancelCause, AgentControlError, AgentHandle, AgentRegistration, AgentRegistry,
    AgentRegistryError, AgentRegistryObserver, AgentSendRequest, AgentStatus, AgentTeardown,
};
use cognit::harness::session_log::HarnessSessionId;

struct MockAgent {
    id: HarnessSessionId,
    session_id: HarnessSessionId,
    status: AtomicU8,
}

impl MockAgent {
    fn new(id: &str) -> Self {
        Self {
            id: HarnessSessionId(id.into()),
            session_id: HarnessSessionId(id.into()),
            status: AtomicU8::new(0),
        }
    }

    fn mismatched(id: &str, session_id: &str) -> Self {
        Self {
            id: HarnessSessionId(id.into()),
            session_id: HarnessSessionId(session_id.into()),
            status: AtomicU8::new(0),
        }
    }
}

#[async_trait]
impl Agent for MockAgent {
    fn id(&self) -> &HarnessSessionId {
        &self.id
    }

    fn session_id(&self) -> &HarnessSessionId {
        &self.session_id
    }

    fn status(&self) -> AgentStatus {
        match self.status.load(Ordering::SeqCst) {
            0 => AgentStatus::Idle,
            1 => AgentStatus::Running,
            _ => AgentStatus::Disposed,
        }
    }

    fn send(&self, _request: AgentSendRequest) -> Result<bool, AgentControlError> {
        Ok(true)
    }

    async fn cancel(&self, _cause: AgentCancelCause) -> Result<(), AgentControlError> {
        self.status.store(0, Ordering::SeqCst);
        Ok(())
    }

    async fn when_idle(&self) -> Result<(), AgentControlError> {
        Ok(())
    }
}

struct RecordingObserver {
    events: Arc<Mutex<Vec<String>>>,
    detach_during_created: Mutex<Option<AgentRegistration>>,
}

impl RecordingObserver {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            events,
            detach_during_created: Mutex::new(None),
        }
    }
}

impl AgentRegistryObserver for RecordingObserver {
    fn created(&self, agent: Arc<dyn Agent>) {
        self.events
            .lock()
            .unwrap()
            .push(format!("created:{}", agent.id().0));
        if let Some(registration) = self.detach_during_created.lock().unwrap().as_ref() {
            registration.detach().unwrap();
        }
    }

    fn disposed(&self, id: &HarnessSessionId) {
        self.events
            .lock()
            .unwrap()
            .push(format!("disposed:{}", id.0));
    }
}

struct CountingTeardown {
    calls: AtomicUsize,
    events: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl AgentTeardown for CountingTeardown {
    async fn stop_and_quiesce(&self) -> Result<(), AgentControlError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.events.lock().unwrap().push("quiesced".into());
        tokio::task::yield_now().await;
        Ok(())
    }
}

#[test]
fn enter_is_unpublished_and_collision_and_identity_checks_are_authoritative() {
    let registry = AgentRegistry::new();
    let agent: Arc<dyn Agent> = Arc::new(MockAgent::new("agent-1"));
    let registration = registry.enter(agent.clone(), None).unwrap();
    assert_eq!(registry.list().unwrap().len(), 1);
    assert_eq!(registry.roots().unwrap().len(), 1);
    assert_eq!(registration.id().0, "agent-1");

    assert!(matches!(
        registry.enter(agent, None),
        Err(AgentRegistryError::Collision(id)) if id == "agent-1"
    ));
    assert!(matches!(
        registry.enter(Arc::new(MockAgent::mismatched("a", "s")), None),
        Err(AgentRegistryError::IdentityMismatch)
    ));
}

#[test]
fn created_listener_detach_is_deferred_and_preserves_event_pairing() {
    let registry = AgentRegistry::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observer = Arc::new(RecordingObserver::new(events.clone()));
    registry.observe(observer.clone()).unwrap();
    let registration = registry
        .enter(Arc::new(MockAgent::new("agent-2")), None)
        .unwrap();
    *observer.detach_during_created.lock().unwrap() = Some(registration.clone());

    registration.announce().unwrap();
    assert!(registry
        .get(&HarnessSessionId("agent-2".into()))
        .unwrap()
        .is_none());
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["created:agent-2", "disposed:agent-2"]
    );
}

#[tokio::test]
async fn handle_disposal_is_memoized_and_detaches_only_after_quiescence() {
    let registry = AgentRegistry::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    registry
        .observe(Arc::new(RecordingObserver::new(events.clone())))
        .unwrap();
    let agent: Arc<dyn Agent> = Arc::new(MockAgent::new("agent-3"));
    let registration = registry.enter(agent.clone(), None).unwrap();
    registration.announce().unwrap();
    let teardown = Arc::new(CountingTeardown {
        calls: AtomicUsize::new(0),
        events: events.clone(),
    });
    let handle = Arc::new(AgentHandle::new(agent, teardown.clone(), registration));

    let left = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.dispose().await })
    };
    let right = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.dispose().await })
    };
    left.await.unwrap().unwrap();
    right.await.unwrap().unwrap();

    assert_eq!(teardown.calls.load(Ordering::SeqCst), 1);
    assert!(registry
        .get(&HarnessSessionId("agent-3".into()))
        .unwrap()
        .is_none());
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["created:agent-3", "quiesced", "disposed:agent-3"]
    );
}

#[test]
fn stale_registration_cannot_remove_same_id_replacement() {
    let registry = AgentRegistry::new();
    let stale = registry
        .enter(Arc::new(MockAgent::new("agent-4")), None)
        .unwrap();
    assert!(stale.detach().unwrap());
    let replacement = registry
        .enter(Arc::new(MockAgent::new("agent-4")), None)
        .unwrap();
    replacement.announce().unwrap();

    assert!(!stale.detach().unwrap());
    assert!(registry
        .get(&HarnessSessionId("agent-4".into()))
        .unwrap()
        .is_some());
}

#[test]
fn runtime_ownership_is_independent_of_session_lineage() {
    let registry = AgentRegistry::new();
    let owner = HarnessSessionId("parent".into());
    registry
        .enter(Arc::new(MockAgent::new("child")), Some(&owner))
        .unwrap();
    assert!(registry
        .is_owned_by(&HarnessSessionId("child".into()), &owner)
        .unwrap());
    assert!(registry.roots().unwrap().is_empty());
}
