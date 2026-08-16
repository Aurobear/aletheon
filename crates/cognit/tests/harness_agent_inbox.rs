use std::sync::{Arc, Mutex};

use cognit::harness::agent::{AgentInbox, AgentInboxError, AgentSendRequest};
use cognit::harness::session_log::{
    HarnessSessionEventKind, HarnessSessionId, HarnessSessionLog, InboxSpliceOutcome, InboxTarget,
};
use contracts::Message;
use kernel::chronos::TestClock;

fn inbox() -> (Arc<Mutex<HarnessSessionLog>>, AgentInbox) {
    let session = Arc::new(Mutex::new(
        HarnessSessionLog::new(HarnessSessionId("agent-inbox".into())).unwrap(),
    ));
    let inbox = AgentInbox::new(session.clone(), Arc::new(TestClock::new(100, 0)));
    (session, inbox)
}

#[test]
fn followup_steer_and_inject_share_one_ordered_inbox_contract() {
    let (session, inbox) = inbox();
    assert!(!inbox
        .send(AgentSendRequest::inject(
            "inject-1",
            Message::user("context")
        ))
        .unwrap());
    assert!(inbox
        .send(AgentSendRequest::steer(
            "steer-1",
            Message::user("correction")
        ))
        .unwrap());
    assert!(inbox
        .send(AgentSendRequest::followup(
            "turn-1",
            Message::user("first prompt")
        ))
        .unwrap());
    assert!(inbox
        .send(AgentSendRequest::followup(
            "turn-2",
            Message::user("second prompt")
        ))
        .unwrap());

    let first = inbox.claim_for_turn().unwrap();
    assert!(first.wake_requested);
    assert_eq!(
        first
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["inject-1", "steer-1", "turn-1"]
    );

    let second = inbox.claim_for_turn().unwrap();
    assert_eq!(second.messages.len(), 1);
    assert_eq!(second.messages[0].id, "turn-2");
    assert!(inbox.is_empty().unwrap());

    let log = session.lock().unwrap();
    assert!(log.derive_messages().is_empty());
    assert!(log.events().iter().any(|event| matches!(
        event.kind,
        HarnessSessionEventKind::InboxSpliced {
            target: InboxTarget::NextStep,
            removed_count: 2,
            outcome: Some(InboxSpliceOutcome::Claimed),
            ..
        }
    )));
}

#[test]
fn duplicate_ids_fail_without_mutating_or_logging() {
    let (session, inbox) = inbox();
    inbox
        .send(AgentSendRequest::followup("same", Message::user("one")))
        .unwrap();
    let before = session.lock().unwrap().events().len();
    let error = inbox
        .send(AgentSendRequest::steer("same", Message::user("two")))
        .unwrap_err();
    assert_eq!(error, AgentInboxError::DuplicateMessageId("same".into()));
    assert_eq!(session.lock().unwrap().events().len(), before);
    let claimed = inbox.claim_for_turn().unwrap();
    assert_eq!(claimed.messages.len(), 1);
    assert_eq!(claimed.messages[0].id, "same");
}

#[test]
fn cancellation_discards_both_queues_with_typed_events() {
    let (session, inbox) = inbox();
    inbox
        .send(AgentSendRequest::inject("i", Message::user("context")))
        .unwrap();
    inbox
        .send(AgentSendRequest::followup("f", Message::user("work")))
        .unwrap();
    let discarded = inbox.discard_all().unwrap();
    assert_eq!(discarded.len(), 2);
    assert!(inbox.is_empty().unwrap());

    let log = session.lock().unwrap();
    let cancellations = log
        .events()
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                HarnessSessionEventKind::InboxSpliced {
                    outcome: Some(InboxSpliceOutcome::Cancelled),
                    ..
                }
            )
        })
        .count();
    assert_eq!(cancellations, 2);
}
