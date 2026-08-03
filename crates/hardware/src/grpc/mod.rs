//! Vendor-neutral embodiment gateway gRPC client contract.
//!
//! All gRPC types are confined to this module. Nothing outside `hardware::grpc`
//! may import protobuf generated types directly.

pub mod convert;
pub mod error;
pub mod provider;

/// SHA-256 digest of the canonical `gateway.proto` this client speaks. Locked
/// by `grpc_contract.rs` against the bridge's copy; recorded on episode reports
/// so a report is self-describing about the wire protocol it ran under.
pub const BRIDGE_PROTOCOL_DIGEST: &str =
    "4a205a75ac7643d7769fbd7bd52f32faba64b4cdf9d81f6908617da490a7d7ff";

/// Generated wire types from `gateway.proto`.
/// Public only for integration test accessibility; proto types must not
/// leak into other crates.
pub mod wire {
    tonic::include_proto!("aletheon.embodiment.gateway.v1");
}
