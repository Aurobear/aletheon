//! Visual-grounding port owned by the ACIX tool domain.

use crate::drivers::types::{Bounds, Image};
use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct GroundingResult {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub confidence: f32,
    pub label: String,
}

impl GroundingResult {
    pub fn bounds(&self) -> Bounds {
        Bounds {
            x: self.x - self.width / 2,
            y: self.y - self.height / 2,
            width: self.width,
            height: self.height,
        }
    }

    pub fn center(&self) -> (i32, i32) {
        (self.x, self.y)
    }
}

#[async_trait]
pub trait GroundingProvider: Send + Sync {
    async fn locate(&self, image: &Image, description: &str) -> Result<GroundingResult>;

    async fn locate_all(&self, image: &Image, description: &str) -> Result<Vec<GroundingResult>> {
        Ok(vec![self.locate(image, description).await?])
    }
}

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
            label: "mock".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_grounding_uses_image_center() {
        let image = Image {
            width: 1920,
            height: 1080,
            data: vec![],
        };
        let result = MockGroundingProvider
            .locate(&image, "anything")
            .await
            .unwrap();
        assert_eq!(result.center(), (960, 540));
        assert_eq!(result.bounds().x, 910);
    }
}
