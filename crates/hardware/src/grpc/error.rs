//! Stable error-code mapping from wire to domain.
//!
//! Maps gRPC `ErrorCode` enum variants to `ProviderError` without parsing
//! Display strings. Unknown wire enum values are treated as protocol rejection.

use crate::grpc::wire;
use crate::ProviderError;

/// Map a wire ErrorCode to a domain ProviderError.
///
/// Unknown or unspecified codes map to `Rejected` to fail closed.
pub fn map_error(error: &wire::ErrorDetail) -> ProviderError {
    match wire::ErrorCode::try_from(error.code) {
        Ok(wire::ErrorCode::ProviderDisconnected) => ProviderError::Disconnected,
        Ok(wire::ErrorCode::DeadlineExceeded) => ProviderError::Timeout,
        Ok(wire::ErrorCode::InvalidArgument)
        | Ok(wire::ErrorCode::UnsupportedVersion)
        | Ok(wire::ErrorCode::UnknownDevice)
        | Ok(wire::ErrorCode::UnknownSkill)
        | Ok(wire::ErrorCode::Conflict)
        | Ok(wire::ErrorCode::NotReady)
        | Ok(wire::ErrorCode::StaleState)
        | Ok(wire::ErrorCode::Internal)
        | Ok(wire::ErrorCode::Unspecified) => {
            ProviderError::Rejected(error.message.clone())
        }
        Err(_) => {
            // Unknown error code → fail closed as rejected
            ProviderError::Rejected(format!(
                "unknown error code {}: {}",
                error.code, error.message
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_maps_to_disconnected() {
        let detail = wire::ErrorDetail {
            code: wire::ErrorCode::ProviderDisconnected as i32,
            message: "ROS master unreachable".into(),
            ..Default::default()
        };
        assert!(matches!(map_error(&detail), ProviderError::Disconnected));
    }

    #[test]
    fn deadline_exceeded_maps_to_timeout() {
        let detail = wire::ErrorDetail {
            code: wire::ErrorCode::DeadlineExceeded as i32,
            message: "lease expired".into(),
            ..Default::default()
        };
        assert!(matches!(map_error(&detail), ProviderError::Timeout));
    }

    #[test]
    fn unknown_skill_maps_to_rejected() {
        let detail = wire::ErrorDetail {
            code: wire::ErrorCode::UnknownSkill as i32,
            message: "skill not registered".into(),
            ..Default::default()
        };
        assert!(matches!(map_error(&detail), ProviderError::Rejected(_)));
    }

    #[test]
    fn unknown_error_code_fails_closed() {
        let detail = wire::ErrorDetail {
            code: 999,
            message: "unexpected".into(),
            ..Default::default()
        };
        assert!(matches!(map_error(&detail), ProviderError::Rejected(_)));
    }
}
