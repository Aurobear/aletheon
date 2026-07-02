use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;

use base::message::{ContentBlock, ImageSource, Message, Role};
use base::types::grounding::{GroundingProvider, GroundingResult};
use base::types::vision::Image;

use crate::r#impl::llm::provider::LlmProvider;

/// Vision-based grounding provider that uses a multimodal LLM to locate
/// UI elements by natural language description.
pub struct VisionGroundingProvider {
    llm: Arc<dyn LlmProvider>,
}

impl VisionGroundingProvider {
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl GroundingProvider for VisionGroundingProvider {
    async fn locate(&self, image: &Image, description: &str) -> Result<GroundingResult> {
        let (media_type, base64_data) = image
            .to_base64_png()
            .context("Failed to encode image to base64 PNG")?;

        let prompt = format!(
            "You are a UI element locator. Look at this screenshot and find the element described as: \"{description}\"

Return ONLY a JSON object (no markdown, no explanation) with this exact format:
{{\"x\": <center_x_pixel>, \"y\": <center_y_pixel>, \"width\": <width_pixels>, \"height\": <height_pixels>, \"confidence\": <0.0_to_1.0>, \"label\": \"<element_name>\"}}

The image is {width}x{height} pixels. Coordinates are in pixels from top-left corner.
If you cannot find the element, return confidence 0.0 and coordinates 0,0.",
            description = description,
            width = image.width,
            height = image.height,
        );

        let messages = vec![Message {
            role: Role::User,
            content: vec![
                ContentBlock::Image {
                    source: ImageSource::Base64 {
                        media_type,
                        data: base64_data,
                    },
                },
                ContentBlock::Text { text: prompt },
            ],
        }];

        let response = self
            .llm
            .complete(&messages, &[])
            .await
            .context("LLM grounding request failed")?;

        // Extract text from response
        let text = response
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Parse JSON from response (handle possible markdown code fences)
        let json_str = extract_json(&text);

        let parsed: serde_json::Value = serde_json::from_str(json_str)
            .context(format!("Failed to parse grounding JSON: {text}"))?;

        Ok(GroundingResult {
            x: parsed["x"].as_i64().unwrap_or(0) as i32,
            y: parsed["y"].as_i64().unwrap_or(0) as i32,
            width: parsed["width"].as_i64().unwrap_or(0) as i32,
            height: parsed["height"].as_i64().unwrap_or(0) as i32,
            confidence: parsed["confidence"].as_f64().unwrap_or(0.0) as f32,
            label: parsed["label"].as_str().unwrap_or("").to_string(),
        })
    }
}

/// Extract JSON from a string that might be wrapped in markdown code fences.
fn extract_json(text: &str) -> &str {
    let trimmed = text.trim();
    // Try to find JSON between ``` markers
    if let Some(start) = trimmed.find("```json") {
        let json_start = start + 7;
        if let Some(end) = trimmed[json_start..].find("```") {
            return trimmed[json_start..json_start + end].trim();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let json_start = start + 3;
        // Skip optional language identifier on same line
        let json_start = trimmed[json_start..]
            .find('\n')
            .map(|n| json_start + n + 1)
            .unwrap_or(json_start);
        if let Some(end) = trimmed[json_start..].find("```") {
            return trimmed[json_start..json_start + end].trim();
        }
    }
    // Try to find raw JSON object
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return &trimmed[start..=end];
        }
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_raw() {
        let text = r#"{"x": 100, "y": 200, "width": 50, "height": 30, "confidence": 0.95, "label": "button"}"#;
        let json = extract_json(text);
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed["x"], 100);
    }

    #[test]
    fn test_extract_json_code_fence() {
        let text = "Here is the result:\n```json\n{\"x\": 50, \"y\": 60, \"width\": 10, \"height\": 20, \"confidence\": 0.8, \"label\": \"icon\"}\n```";
        let json = extract_json(text);
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed["x"], 50);
    }

    #[test]
    fn test_extract_json_code_fence_no_lang() {
        let text = "```\n{\"x\": 10, \"y\": 20, \"width\": 5, \"height\": 5, \"confidence\": 0.5, \"label\": \"x\"}\n```";
        let json = extract_json(text);
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed["x"], 10);
    }
}
