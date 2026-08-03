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
//! manifest, and fresh monotonic observations. The execute/verify path is
//! covered in CI by the `SimulatedKuavo` in-process tests.

use hardware::grpc::provider::{GrpcEmbodimentProvider, GrpcProviderConfig};
use hardware::EmbodimentProvider;
use fabric::types::embodiment::DeviceId;

const BRIDGE_ENDPOINT: &str = "http://127.0.0.1:50051";
const DEVICE_ID: &str = "kuavo-mujoco-01";

async fn connect() -> GrpcEmbodimentProvider {
    GrpcEmbodimentProvider::connect(GrpcProviderConfig {
        endpoint: BRIDGE_ENDPOINT.into(),
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
    assert!(
        !skills.is_empty(),
        "bridge must expose at least one skill"
    );
    assert!(
        skills.iter().any(|s| s.skill.0 == "kuavo.stop"),
        "bridge must expose the read-only safe skill kuavo.stop, got: {:?}",
        skills.iter().map(|s| s.skill.0.as_str()).collect::<Vec<_>>()
    );
}

#[ignore]
#[tokio::test]
async fn bridge_observation_is_fresh_and_monotonic() {
    let provider = connect().await;
    let device = DeviceId(DEVICE_ID.into());
    let first = provider
        .get_state(&device)
        .await
        .expect("get_state over bridge")
        .expect("device has state");
    let second = provider
        .get_state(&device)
        .await
        .expect("get_state over bridge")
        .expect("device has state");
    assert!(
        second.sequence > first.sequence,
        "observation sequence must be monotonic"
    );
    assert!(
        !first.valid_until.is_some_and(|deadline| deadline.is_expired_at(first.source_time)),
        "observation must carry a live validity window"
    );
}
