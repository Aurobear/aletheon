//! Bounded visual frame reference — no image bytes in the contract.
//! FrameRef carries a URI/hash reference; image bytes live in the artifact store.

use serde::{Deserialize, Serialize};

const ALLOWED_MIME_TYPES: &[&str] = &["image/jpeg", "image/png"];
const MAX_DIMENSION: u32 = 8192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRef {
    /// Content-addressed URI (e.g. "artifact://sha256:abc123").
    pub uri: String,
    /// SHA-256 hex digest of the image bytes.
    pub sha256: String,
    /// MIME type — must be image/jpeg or image/png.
    pub mime_type: String,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Wall-clock timestamp when the frame was captured.
    pub source_time_ms: i64,
    /// Human-readable camera identifier.
    pub camera_id: String,
    /// Monotonic frame sequence from the camera.
    pub frame_id: u64,
}

impl FrameRef {
    pub fn validate(&self) -> Result<(), String> {
        if !ALLOWED_MIME_TYPES.contains(&self.mime_type.as_str()) {
            return Err(format!("unsupported MIME type: {}", self.mime_type));
        }
        if self.width == 0 || self.height == 0 {
            return Err("dimensions must be positive".into());
        }
        if self.width > MAX_DIMENSION || self.height > MAX_DIMENSION {
            return Err(format!("dimensions exceed max {MAX_DIMENSION}"));
        }
        if self.sha256.len() != 64 || !self.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("sha256 must be a 64-char hex string".into());
        }
        if self.uri.starts_with("http://")
            || self.uri.starts_with("https://")
            || self.uri.starts_with("data:")
        {
            return Err(format!("untrusted URI scheme: {}", self.uri));
        }
        if self.uri.contains("..") {
            return Err("URI must not contain path traversal".into());
        }
        Ok(())
    }

    pub fn is_expired(&self, now_ms: i64, max_age_ms: i64) -> bool {
        now_ms - self.source_time_ms > max_age_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_frame_passes_validation() {
        let f = FrameRef {
            uri:
                "artifact://sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                    .into(),
            sha256: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".into(),
            mime_type: "image/jpeg".into(),
            width: 640,
            height: 480,
            source_time_ms: 1000,
            camera_id: "cam0".into(),
            frame_id: 1,
        };
        assert!(f.validate().is_ok());
    }

    #[test]
    fn png_is_allowed() {
        let f = FrameRef {
            uri: "artifact://sha256:aaaa".into(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            mime_type: "image/png".into(),
            width: 1,
            height: 1,
            source_time_ms: 0,
            camera_id: "c".into(),
            frame_id: 0,
        };
        assert!(f.validate().is_ok());
    }

    #[test]
    fn unsupported_mime_rejected() {
        let f = FrameRef {
            mime_type: "image/gif".into(),
            ..valid_frame()
        };
        assert!(f.validate().is_err());
    }

    #[test]
    fn data_uri_rejected() {
        let f = FrameRef {
            uri: "data:image/jpeg;base64,...".into(),
            ..valid_frame()
        };
        assert!(f.validate().is_err());
    }

    #[test]
    fn http_uri_rejected() {
        let f = FrameRef {
            uri: "http://example.com/img.jpg".into(),
            ..valid_frame()
        };
        assert!(f.validate().is_err());
    }

    #[test]
    fn zero_dimensions_rejected() {
        let f = FrameRef {
            width: 0,
            height: 0,
            ..valid_frame()
        };
        assert!(f.validate().is_err());
    }

    #[test]
    fn bad_sha256_rejected() {
        let f = FrameRef {
            sha256: "short".into(),
            ..valid_frame()
        };
        assert!(f.validate().is_err());
    }

    fn valid_frame() -> FrameRef {
        FrameRef {
            uri: "artifact://sha256:a".into(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            mime_type: "image/jpeg".into(),
            width: 640,
            height: 480,
            source_time_ms: 1000,
            camera_id: "c".into(),
            frame_id: 0,
        }
    }
}
