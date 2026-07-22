//! Vendor-neutral embodiment gateway gRPC client contract.
//!
//! All gRPC types are confined to this module. Nothing outside `hardware::grpc`
//! may import protobuf generated types directly.

/// Generated wire types from `gateway.proto`.
/// Public only for integration test accessibility; proto types must not
/// leak into other crates.
pub mod wire {
    tonic::include_proto!("aletheon.embodiment.gateway.v1");
}
