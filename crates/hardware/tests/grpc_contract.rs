//! Contract test verifying the hardware proto copy matches the Bridge canonical proto.

use hardware::grpc::BRIDGE_PROTOCOL_DIGEST;
use sha2::{Digest, Sha256};

#[test]
fn proto_copy_hash_matches_bridge_canonical() {
    let proto_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/proto/aletheon/embodiment/gateway/v1/gateway.proto"
    );
    let contents =
        std::fs::read_to_string(proto_path).expect("gateway.proto must exist in hardware/proto/");

    let mut hasher = Sha256::new();
    hasher.update(contents.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    assert_eq!(
        hash, BRIDGE_PROTOCOL_DIGEST,
        "gateway.proto hash mismatch: the hardware copy must match the Bridge canonical.\n\
         Copy the proto from aletheon-kuavo-bridge/proto/ and update BRIDGE_PROTOCOL_DIGEST."
    );
}

#[test]
fn generated_types_compile() {
    // Verify that generated types exist and compile.
    // Health state enum values must be stable.
    let ready = hardware::grpc::wire::HealthState::Ready;
    let degraded = hardware::grpc::wire::HealthState::Degraded;
    let unavailable = hardware::grpc::wire::HealthState::Unavailable;
    assert_eq!(ready as i32, 1);
    assert_eq!(degraded as i32, 2);
    assert_eq!(unavailable as i32, 3);
}

#[test]
fn client_stub_is_available() {
    // Verify the gRPC client stub is generated.
    // If this compiles, tonic-build successfully generated the client.
    let _ = std::mem::size_of::<
        hardware::grpc::wire::embodiment_gateway_client::EmbodimentGatewayClient<
            tonic::transport::Channel,
        >,
    >();
}
