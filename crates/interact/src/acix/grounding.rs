use corpus::drivers::driver::types::{Bounds, Image};
use anyhow::Result;
use async_trait::async_trait;

/// Result of a visual grounding operation.
#[derive(Debug, Clone)]
pub struct GroundingResult {
    /// X coordinate of the element center
    pub x: i32,
    /// Y coordinate of the element center
    pub y: i32,
    /// Width of the bounding box
    pub width: i32,
    /// Height of the bounding box
    pub height: i32,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Human-readable label of what was found
    pub label: String,
}

impl GroundingResult {
    /// Get the bounding box as a Bounds struct
    pub fn bounds(&self) -> Bounds {
        Bounds {
            x: self.x - self.width / 2,
            y: self.y - self.height / 2,
            width: self.width,
            height: self.height,
        }
    }

    /// Get the center point
    pub fn center(&self) -> (i32, i32) {
        (self.x, self.y)
    }
}

/// Provider for visual grounding — locating UI elements by natural language description.
///
/// Visual grounding provider — locates UI elements by natural language description.
///
/// Implemented by the runtime layer to forward to a vision-capable LLM provider.
#[async_trait]
pub trait GroundingProvider: Send + Sync {
    /// Locate an element in the given screenshot by natural language description.
    async fn locate(&self, image: &Image, description: &str) -> Result<GroundingResult>;

    /// Locate multiple elements matching the description.
    /// Default implementation returns a single result.
    async fn locate_all(&self, image: &Image, description: &str) -> Result<Vec<GroundingResult>> {
        let result = self.locate(image, description).await?;
        Ok(vec![result])
    }
}

/// Mock grounding provider for testing.
///
/// Returns the center of the image with confidence 0.0 and label "mock".
pub struct MockGroundingProvider;

#[async_trait]
impl GroundingProvider for MockGroundingProvider {
    async fn locate(&self, image: &Image, _description: &str) -> Result<GroundingResult> {
        Ok(GroundingResult {
            x: (image.width / 2) as i32,
            y: (image.height / 2) as i32,
            width: 100,
            height: 50,
            confidence: 0.0,
            label: "mock".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_grounding_center() {
        let provider = MockGroundingProvider;
        let image = Image {
            width: 1920,
            height: 1080,
            data: vec![0u8; 1920 * 1080 * 3],
        };
        let result = provider.locate(&image, "anything").await.unwrap();
        assert_eq!(result.x, 960);
        assert_eq!(result.y, 540);
        assert_eq!(result.confidence, 0.0);
        assert_eq!(result.label, "mock");
    }

    #[tokio::test]
    async fn test_grounding_result_bounds() {
        let result = GroundingResult {
            x: 500,
            y: 300,
            width: 120,
            height: 40,
            confidence: 0.9,
            label: "button".to_string(),
        };
        let bounds = result.bounds();
        assert_eq!(bounds.x, 440);
        assert_eq!(bounds.y, 280);
        assert_eq!(bounds.width, 120);
        assert_eq!(bounds.height, 40);
        assert_eq!(result.center(), (500, 300));
    }
}
