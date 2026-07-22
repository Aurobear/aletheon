use std::path::PathBuf;

use mnemosyne::embodied_episode::EmbodiedEpisodeRepository;
use fabric::types::embodiment::{DeviceId, SkillId};
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::OperationId;

fn temp_db() -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!(
            "mnemosyne-embodied-episode-test-{}",
            uuid::Uuid::new_v4()
        ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("episodes.db")
}

#[test]
fn create_and_close_episode() {
    let db = temp_db();
    let repo = EmbodiedEpisodeRepository::open(&db).unwrap();
    let op = OperationId::new();
    repo.create_episode("ep1", "goal1", &DeviceId("bot".into()))
        .unwrap();
    repo.append_attempt(
        "ep1",
        1,
        &op,
        &SkillId("stance".into()),
        &ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 5000,
        },
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    repo.close_episode("ep1", "completed").unwrap();

    let loaded = repo.load_episode("ep1").unwrap().unwrap();
    assert_eq!(loaded.goal_id, "goal1");
    assert_eq!(loaded.attempts.len(), 1);
    assert_eq!(loaded.attempts[0].operation_id, op);
}

#[test]
fn idempotent_replay() {
    let db = temp_db();
    let repo = EmbodiedEpisodeRepository::open(&db).unwrap();
    let op = OperationId::new();
    repo.create_episode("ep2", "goal2", &DeviceId("bot".into()))
        .unwrap();

    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals {
            path: "x".into(),
            value: serde_json::json!(0),
        },
        freshness_ms: 0,
        stable_window_ms: 0,
        timeout_ms: 0,
    };

    // Same (episode_id, attempt_number, operation_id) → idempotent
    repo.append_attempt(
        "ep2", 1, &op, &SkillId("s".into()), &eo, None, None, None, None, None,
    )
    .unwrap();

    repo.append_attempt(
        "ep2", 1, &op, &SkillId("s".into()), &eo, None, None, None, None, None,
    )
    .unwrap();

    let loaded = repo.load_episode("ep2").unwrap().unwrap();
    assert_eq!(loaded.attempts.len(), 1);
}

#[test]
fn conflicting_replay_rejected() {
    let db = temp_db();
    let repo = EmbodiedEpisodeRepository::open(&db).unwrap();
    let op1 = OperationId::new();
    let op2 = OperationId::new();
    repo.create_episode("ep3", "goal3", &DeviceId("bot".into()))
        .unwrap();

    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals {
            path: "x".into(),
            value: serde_json::json!(0),
        },
        freshness_ms: 0,
        stable_window_ms: 0,
        timeout_ms: 0,
    };

    repo.append_attempt(
        "ep3", 1, &op1, &SkillId("s".into()), &eo, None, None, None, None, None,
    )
    .unwrap();

    let result = repo.append_attempt(
        "ep3", 1, &op2, &SkillId("s".into()), &eo, None, None, None, None, None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("conflicting replay"));
}

#[test]
fn missing_episode_returns_none() {
    let db = temp_db();
    let repo = EmbodiedEpisodeRepository::open(&db).unwrap();
    assert!(repo.load_episode("nonexistent").unwrap().is_none());
}

#[test]
fn multiple_attempts_ordered() {
    let db = temp_db();
    let repo = EmbodiedEpisodeRepository::open(&db).unwrap();
    let op1 = OperationId::new();
    let op2 = OperationId::new();
    repo.create_episode("ep5", "goal5", &DeviceId("bot".into()))
        .unwrap();

    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals {
            path: "x".into(),
            value: serde_json::json!(0),
        },
        freshness_ms: 0,
        stable_window_ms: 0,
        timeout_ms: 0,
    };

    repo.append_attempt(
        "ep5", 1, &op1, &SkillId("s".into()), &eo, None, None, None, None, None,
    )
    .unwrap();
    repo.append_attempt(
        "ep5", 2, &op2, &SkillId("s".into()), &eo, None, None, None, None, None,
    )
    .unwrap();

    let loaded = repo.load_episode("ep5").unwrap().unwrap();
    assert_eq!(loaded.attempts.len(), 2);
    assert_eq!(loaded.attempts[0].operation_id, op1);
    assert_eq!(loaded.attempts[1].operation_id, op2);
}

#[test]
fn close_nonexistent_episode_fails() {
    let db = temp_db();
    let repo = EmbodiedEpisodeRepository::open(&db).unwrap();
    let result = repo.close_episode("nonexistent", "completed");
    assert!(result.is_err());
}
