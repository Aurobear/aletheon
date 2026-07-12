use crate::drivers::types::{Bounds, Image, OcrResult, OcrWord};
use anyhow::Result;

#[cfg(feature = "ocr-tesseract")]
pub mod tesseract;

/// OCR driver trait
pub trait OcrDriver: Send + Sync {
    /// Perform OCR on an image
    fn recognize(&self, image: &Image) -> Result<OcrResult>;
}

/// Mock OCR driver for testing
pub struct MockOcrDriver;

impl OcrDriver for MockOcrDriver {
    fn recognize(&self, _image: &Image) -> Result<OcrResult> {
        Ok(OcrResult {
            text: "Mock OCR text".into(),
            words: vec![
                OcrWord {
                    text: "Mock".into(),
                    bounds: Bounds {
                        x: 10,
                        y: 10,
                        width: 50,
                        height: 20,
                    },
                    confidence: 0.95,
                },
                OcrWord {
                    text: "OCR".into(),
                    bounds: Bounds {
                        x: 70,
                        y: 10,
                        width: 30,
                        height: 20,
                    },
                    confidence: 0.92,
                },
                OcrWord {
                    text: "text".into(),
                    bounds: Bounds {
                        x: 110,
                        y: 10,
                        width: 40,
                        height: 20,
                    },
                    confidence: 0.88,
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_ocr() {
        let driver = MockOcrDriver;
        let img = Image {
            width: 100,
            height: 50,
            data: vec![0; 15000],
        };
        let result = driver.recognize(&img).unwrap();
        assert_eq!(result.words.len(), 3);
        assert!(result.words[0].confidence > 0.9);
    }

    #[test]
    fn test_mock_ocr_text() {
        let driver = MockOcrDriver;
        let img = Image {
            width: 10,
            height: 10,
            data: vec![0; 300],
        };
        let result = driver.recognize(&img).unwrap();
        assert_eq!(result.text, "Mock OCR text");
        assert_eq!(result.words[0].text, "Mock");
        assert_eq!(result.words[1].text, "OCR");
        assert_eq!(result.words[2].text, "text");
    }

    #[test]
    fn test_mock_ocr_word_bounds() {
        let driver = MockOcrDriver;
        let img = Image {
            width: 10,
            height: 10,
            data: vec![0; 300],
        };
        let result = driver.recognize(&img).unwrap();
        assert_eq!(result.words[0].bounds.x, 10);
        assert_eq!(result.words[0].bounds.y, 10);
        assert_eq!(result.words[1].bounds.x, 70);
        assert_eq!(result.words[2].bounds.x, 110);
    }

    #[test]
    fn test_mock_ocr_confidence_range() {
        let driver = MockOcrDriver;
        let img = Image {
            width: 10,
            height: 10,
            data: vec![0; 300],
        };
        let result = driver.recognize(&img).unwrap();
        for word in &result.words {
            assert!(word.confidence > 0.0 && word.confidence <= 1.0);
        }
    }
}
