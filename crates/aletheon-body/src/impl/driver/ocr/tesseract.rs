use anyhow::{Context, Result};
use tracing::debug;

use super::OcrDriver;
use crate::r#impl::driver::types::{Bounds, Image, OcrResult, OcrWord};

/// Tesseract-based OCR driver.
///
/// Requires the `ocr-tesseract` feature and Tesseract libraries
/// (libleptonica, libtesseract) installed on the system.
pub struct TesseractOcrDriver {
    lang: String,
}

impl TesseractOcrDriver {
    /// Create a new driver with English as the default language.
    pub fn new() -> Result<Self> {
        Self::with_lang("eng")
    }

    /// Create a new driver with a specific language (e.g. "eng", "chi_sim").
    pub fn with_lang(lang: &str) -> Result<Self> {
        Ok(Self {
            lang: lang.to_string(),
        })
    }
}

impl OcrDriver for TesseractOcrDriver {
    fn recognize(&self, image: &Image) -> Result<OcrResult> {
        use tesseract::Tesseract;

        let bytes_per_line = (image.width * 3) as i32;

        // Build the Tesseract instance, set raw pixel frame, and run OCR.
        let mut tess = Tesseract::new(None, Some(&self.lang))
            .context("Failed to initialize Tesseract")?
            .set_frame(
                &image.data,
                image.width as i32,
                image.height as i32,
                3, // bytes per pixel (RGB)
                bytes_per_line,
            )
            .context("Failed to set image frame in Tesseract")?
            .recognize()
            .context("Tesseract recognition failed")?;

        let text = tess.get_text().context("Failed to get OCR text")?;

        // Parse TSV output for word-level bounding boxes and confidence.
        // TSV columns: level, page_num, block_num, par_num, line_num, word_num,
        //              left, top, width, height, conf, text
        let tsv = tess
            .get_tsv_text(0)
            .context("Failed to get TSV text from Tesseract")?;

        let words = parse_tsv_words(&tsv);

        debug!(
            "Tesseract OCR: {} words, text length {}",
            words.len(),
            text.len()
        );

        Ok(OcrResult { text, words })
    }
}

/// Parse Tesseract TSV output to extract word-level bounding boxes and confidence.
///
/// TSV format (tab-separated, one row per element):
///   level page_num block_num par_num line_num word_num left top width height conf text
///
/// Word-level entries have `level == 5`.
fn parse_tsv_words(tsv: &str) -> Vec<OcrWord> {
    let mut words = Vec::new();

    for line in tsv.lines().skip(1) {
        // skip header
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 12 {
            continue;
        }

        let level: i32 = cols[0].trim().parse().unwrap_or(0);
        if level != 5 {
            continue; // only word-level entries
        }

        let text = cols[11].trim();
        if text.is_empty() {
            continue;
        }

        let x = cols[6].trim().parse().unwrap_or(0);
        let y = cols[7].trim().parse().unwrap_or(0);
        let width = cols[8].trim().parse().unwrap_or(0);
        let height = cols[9].trim().parse().unwrap_or(0);
        let conf_raw: f32 = cols[10].trim().parse().unwrap_or(-1.0);

        // Tesseract returns -1 for confidence on some elements; clamp to 0..100
        let confidence = if conf_raw < 0.0 {
            0.0
        } else {
            (conf_raw / 100.0).clamp(0.0, 1.0)
        };

        words.push(OcrWord {
            text: text.to_string(),
            bounds: Bounds {
                x,
                y,
                width,
                height,
            },
            confidence,
        });
    }

    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tsv_words() {
        // Sample TSV output from Tesseract (header + 2 words)
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                    5\t1\t1\t1\t1\t1\t100\t200\t50\t20\t92.5\tHello\n\
                    5\t1\t1\t1\t1\t2\t160\t200\t60\t20\t87.3\tWorld\n";

        let words = parse_tsv_words(tsv);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Hello");
        assert_eq!(words[0].bounds.x, 100);
        assert_eq!(words[0].bounds.y, 200);
        assert_eq!(words[0].bounds.width, 50);
        assert_eq!(words[0].bounds.height, 20);
        assert!((words[0].confidence - 0.925).abs() < 0.01);
        assert_eq!(words[1].text, "World");
        assert!((words[1].confidence - 0.873).abs() < 0.01);
    }

    #[test]
    fn test_parse_tsv_skips_non_word_levels() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                    1\t1\t0\t0\t0\t0\t0\t0\t1000\t500\t-1\t\n\
                    2\t1\t1\t0\t0\t0\t50\t50\t900\t400\t-1\t\n\
                    5\t1\t1\t1\t1\t1\t100\t200\t50\t20\t95.0\tTest\n";

        let words = parse_tsv_words(tsv);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "Test");
    }

    #[test]
    fn test_parse_tsv_handles_negative_confidence() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                    5\t1\t1\t1\t1\t1\t10\t10\t30\t10\t-1\tOops\n";

        let words = parse_tsv_words(tsv);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].confidence, 0.0);
    }
}
