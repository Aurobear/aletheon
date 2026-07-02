//! Vision types -- image representation and codec.
//!
//! Moved from `corpus::drivers::driver::types` (Tier 2c) so that `cognit`
//! (Brain) can depend only on `base`, not on `corpus` (Body).

use serde::{Deserialize, Serialize};

/// RGB image in row-major format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGB bytes, row-major
}

/// Screen bounding box.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Image {
    /// Convert raw RGB image data to base64-encoded PNG.
    /// Returns (media_type, base64_data) suitable for LLM vision APIs.
    pub fn to_base64_png(&self) -> anyhow::Result<(String, String)> {
        use std::io::Cursor;

        let mut png_buf = Vec::new();
        {
            let mut cursor = Cursor::new(&mut png_buf);
            let mut encoder = png::Encoder::new(&mut cursor, self.width, self.height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&self.data)?;
            writer.finish()?;
        }

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
        Ok(("image/png".to_string(), b64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_encodes_to_base64_png() {
        let img = Image {
            width: 2,
            height: 2,
            data: vec![0u8; 2 * 2 * 3],
        };
        let (media_type, b64) = img.to_base64_png().unwrap();
        assert_eq!(media_type, "image/png");
        assert!(!b64.is_empty());
    }

    #[test]
    fn bounds_construction() {
        let b = Bounds {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        assert_eq!(b.width, 10);
        assert_eq!(b.height, 10);
    }
}
