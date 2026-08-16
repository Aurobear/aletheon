//! Cross-repo bridge compatibility fixtures (PR7/PR8).
//!
//! These tests require the `aletheon-kuavo-bridge` (ROS Noetic + MuJoCo)
//! environment and are `#[ignore]` by default. Run explicitly with:
//!
//! ```text
//! bash scripts/cargo-agent.sh test -p hardware --test grpc_cross_repo -- --ignored
//! ```
//!
//! The read-only checks validate the GetCapabilities handshake, the skill
//! manifest, per-schema sequence monotonicity, and explicit Unix/monotonic time
//! separation. The execute/verify path is
//! covered in CI by the `SimulatedKuavo` in-process tests.

use std::collections::HashMap;
use std::time::Duration;

use ::contracts::types::embodiment::{DeviceId, EmbodiedObservation};
use hardware::grpc::provider::{
    GrpcEmbodimentProvider, GrpcProviderConfig, ObservationSchemaRequirement,
};
use hardware::EmbodimentProvider;

const BRIDGE_ENDPOINT: &str = "http://127.0.0.1:50051";
const DEVICE_ID: &str = "kuavo-mujoco-01";

async fn connect() -> GrpcEmbodimentProvider {
    GrpcEmbodimentProvider::connect(GrpcProviderConfig {
        endpoint: BRIDGE_ENDPOINT.into(),
        required_device_id: Some(DEVICE_ID.into()),
        required_observation_schemas: vec![
            ObservationSchemaRequirement {
                schema: "base_pose".into(),
                schema_version: 1,
            },
            ObservationSchemaRequirement {
                schema: "base_twist".into(),
                schema_version: 1,
            },
        ],
        ..Default::default()
    })
    .await
    .expect("bridge must be reachable; run aletheon-kuavo-bridge first")
}

#[ignore]
#[tokio::test]
async fn bridge_capabilities_and_skill_manifest_match_contract() {
    let provider = connect().await;
    let device = DeviceId(DEVICE_ID.into());
    let skills = provider
        .list_skills(&device)
        .await
        .expect("list_skills over bridge");
    // The bridge is in its read-only phase: `kuavo.stop` is the guaranteed
    // skill; `kuavo.move_base_timed` is exposed for bounded base motion.
    // `kuavo.stance` execution is a later bridge phase — asserted as present
    // once the bridge exposes it.
    assert!(!skills.is_empty(), "bridge must expose at least one skill");
    assert!(
        skills.iter().any(|s| s.skill.0 == "kuavo.stop"),
        "bridge must expose the read-only safe skill kuavo.stop, got: {:?}",
        skills
            .iter()
            .map(|s| s.skill.0.as_str())
            .collect::<Vec<_>>()
    );
}

#[ignore]
#[tokio::test]
async fn bridge_observation_is_fresh_and_monotonic() {
    let provider = connect().await;
    let device = DeviceId(DEVICE_ID.into());

    // A live source's `sequence` is a receive counter (the bridge increments it
    // once per ROS callback), so a strict `>` between two instantaneous reads is
    // inherently flaky — any sub-tick stall in the source returns an equal
    // sequence. The real contract is non-decreasing per observation schema plus
    // at least one strict advance across a short window (fresh data arriving).
    // Sample a window instead of two reads, and compare sequences per schema
    // because the bridge's schemas carry independent counters but share a single
    // `source` string.
    const SAMPLES: usize = 5;
    const INTERVAL: Duration = Duration::from_millis(50);
    let mut samples: Vec<Vec<EmbodiedObservation>> = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let observations = provider
            .observe(&device)
            .await
            .expect("observe over bridge");
        assert!(!observations.is_empty(), "bridge must return observations");
        samples.push(observations);
        tokio::time::sleep(INTERVAL).await;
    }

    let mut per_schema: HashMap<String, Vec<u64>> = HashMap::new();
    for sample in &samples {
        for observation in sample {
            assert!(
                observation.received_unix_ms > 1_000_000_000_000,
                "bridge receive timestamp must be Unix milliseconds"
            );
            assert!(
                observation.received_unix_ms >= observation.source_unix_ms,
                "receive time must not precede source time"
            );
            per_schema
                .entry(observation.schema.clone())
                .or_default()
                .push(observation.sequence);
        }
    }
    assert!(!per_schema.is_empty(), "at least one observation schema");

    let mut advanced = false;
    for (schema, sequences) in &per_schema {
        for pair in sequences.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "sequence must be non-decreasing for schema {schema}: {sequences:?}"
            );
            advanced |= pair[1] > pair[0];
        }
    }
    assert!(
        advanced,
        "observation stream must advance (fresh data) within the sample window"
    );
}
