//! Bounded visual frame reference — no image bytes in the contract.
//! FrameRef carries a URI/hash reference; image bytes live in the artifact store.

use serde::{Deserialize, Serialize};

const ALLOWED_MIME_TYPES: &[&str] = &["image/jpeg", "image/png"];
const MAX_DIMENSION: u32 = 8192;
const MAX_FRAME_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRef {
    /// Content-addressed URI (e.g. `artifact://sha256/<digest>`).
    pub uri: String,
    /// SHA-256 hex digest of the image bytes.
    pub sha256: String,
    /// MIME type — must be image/jpeg or image/png.
    pub mime_type: String,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Encoded content size. This is used for input budgeting without reading
    /// or copying the image bytes into a prompt, event, or episode record.
    pub byte_len: u64,
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
        if self.byte_len == 0 || self.byte_len > MAX_FRAME_BYTES {
            return Err(format!(
                "encoded frame size must be within 1..={MAX_FRAME_BYTES} bytes"
            ));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err("sha256 must be a 64-char hex string".into());
        }
        if self.camera_id.trim().is_empty() {
            return Err("camera_id must not be empty".into());
        }
        if let Some(digest) = self.uri.strip_prefix("artifact://sha256/") {
            if digest != self.sha256 {
                return Err("artifact URI digest does not match frame sha256".into());
            }
        } else if let Some(path) = self.uri.strip_prefix("workspace://") {
            if path.is_empty()
                || path.starts_with('/')
                || path
                    .split('/')
                    .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
            {
                return Err("workspace URI must contain a traversal-free relative path".into());
            }
        } else {
            return Err(format!("untrusted URI scheme: {}", self.uri));
        }
        Ok(())
    }

    /// Return the typed scheme after base validation. Runtime policy still
    /// decides which of these schemes/prefixes is authorized for a Robot task.
    pub fn scheme(&self) -> Result<&'static str, String> {
        self.validate()?;
        if self.uri.starts_with("artifact://") {
            Ok("artifact")
        } else {
            Ok("workspace")
        }
    }

    pub fn is_expired(&self, now_ms: i64, max_age_ms: i64) -> bool {
        max_age_ms < 0 || now_ms.saturating_sub(self.source_time_ms) > max_age_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_frame_passes_validation() {
        let f = FrameRef {
            uri:
                "artifact://sha256/abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                    .into(),
            sha256: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".into(),
            mime_type: "image/jpeg".into(),
            width: 640,
            height: 480,
            byte_len: 32_000,
            source_time_ms: 1000,
            camera_id: "cam0".into(),
            frame_id: 1,
        };
        assert!(f.validate().is_ok());
    }

    #[test]
    fn png_is_allowed() {
        let f = FrameRef {
            uri:
                "artifact://sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            mime_type: "image/png".into(),
            width: 1,
            height: 1,
            byte_len: 64,
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
            uri:
                "artifact://sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            mime_type: "image/jpeg".into(),
            width: 640,
            height: 480,
            byte_len: 32_000,
            source_time_ms: 1000,
            camera_id: "c".into(),
            frame_id: 0,
        }
    }
}
