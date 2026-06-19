use std::sync::Arc;

use crate::r#impl::acix::{Aci, GroundingProvider, MockGroundingProvider};
use crate::r#impl::driver::{
    a11y::MockA11yDriver, display::MockDisplayDriver, factory::DriverFactory,
    input::MockInputDriver, ocr::MockOcrDriver,
};
use anyhow::Result;

/// Computer operation command handler.
pub struct ComputerCommands {
    aci: Aci,
    grounding: Arc<dyn GroundingProvider>,
}

impl ComputerCommands {
    /// Create a new instance using real drivers when available, with mock fallback.
    pub fn new() -> Self {
        let input = DriverFactory::try_input().unwrap_or_else(|| Box::new(MockInputDriver::new()));
        let display = DriverFactory::try_display()
            .unwrap_or_else(|| Box::new(MockDisplayDriver::new(1920, 1080)));
        let a11y = DriverFactory::try_a11y().unwrap_or_else(|| Box::new(MockA11yDriver::new()));
        #[cfg(feature = "ocr-tesseract")]
        let ocr = DriverFactory::try_ocr().or_else(|| Some(Box::new(MockOcrDriver)));
        #[cfg(not(feature = "ocr-tesseract"))]
        let ocr: Option<Box<dyn crate::r#impl::driver::ocr::OcrDriver>> =
            Some(Box::new(MockOcrDriver));
        let window = DriverFactory::try_window();
        let clipboard = DriverFactory::try_clipboard();

        Self {
            aci: Aci::new(input, display, a11y, ocr, window, clipboard),
            grounding: Arc::new(MockGroundingProvider),
        }
    }

    /// Create an instance with all mock drivers (for testing).
    pub fn new_mock() -> Self {
        Self {
            aci: Aci::new_basic(
                Box::new(MockInputDriver::new()),
                Box::new(MockDisplayDriver::new(1920, 1080)),
                Box::new(MockA11yDriver::new()),
                Some(Box::new(MockOcrDriver)),
            ),
            grounding: Arc::new(MockGroundingProvider),
        }
    }

    pub fn handle(&self, args: &str) -> Result<String> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        match parts.first() {
            Some(&"screenshot") => {
                let img = self.aci.screenshot()?;
                Ok(format!(
                    "Screenshot: {}x{} ({} bytes)",
                    img.width,
                    img.height,
                    img.data.len()
                ))
            }
            Some(&"tree") => {
                let tree = self.aci.get_accessibility_tree()?;
                Ok(format_tree(&tree.root, 0))
            }
            Some(&"click") => {
                let x: i32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let y: i32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                self.aci.click(x, y)?;
                Ok(format!("Clicked at ({x}, {y})"))
            }
            Some(&"rclick") => {
                let x: i32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let y: i32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                self.aci.right_click(x, y)?;
                Ok(format!("Right-clicked at ({x}, {y})"))
            }
            Some(&"type") => {
                let text = parts[1..].join(" ");
                self.aci.type_text(&text)?;
                Ok(format!("Typed: {text:?}"))
            }
            Some(&"find") => {
                let desc = parts[1..].join(" ");
                let elems = self.aci.find_element(&desc)?;
                if elems.is_empty() {
                    Ok("No elements found".into())
                } else {
                    let mut out = format!("Found {} element(s):\n", elems.len());
                    for (i, e) in elems.iter().enumerate() {
                        out += &format!(
                            "  [{i}] role={} name={:?} bounds={:?}\n",
                            e.role, e.name, e.bounds
                        );
                    }
                    Ok(out)
                }
            }
            Some(&"observe") => {
                let obs = self.aci.smart_observe()?;
                Ok(format!("{obs:#?}"))
            }
            Some(&"ground") => {
                let desc = parts[1..].join(" ");
                if desc.is_empty() {
                    return Err(anyhow::anyhow!("Usage: /computer ground <description>"));
                }
                // Block on async grounding call
                let result = tokio::runtime::Handle::current().block_on(
                    self.aci.locate_element(&desc, &*self.grounding)
                )?;
                Ok(format!(
                    "Located {:?}: center=({}, {}) size={}x{} confidence={:.2}",
                    result.label, result.x, result.y, result.width, result.height, result.confidence
                ))
            }
            Some(&"window") => match parts.get(1).copied() {
                Some("list") => {
                    let wins = self.aci.list_windows()?;
                    if wins.is_empty() {
                        Ok("No windows found".into())
                    } else {
                        let mut out = format!("{} window(s):\n", wins.len());
                        for w in &wins {
                            let focus_tag = if w.focused { " *" } else { "" };
                            out += &format!(
                                "  [{}] {} ({}){}\n",
                                w.id, w.title, w.app_name, focus_tag
                            );
                        }
                        Ok(out)
                    }
                }
                Some("focus") => {
                    let id: u64 = parts
                        .get(2)
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| anyhow::anyhow!("Usage: window focus <id>"))?;
                    self.aci.focus_window(id)?;
                    Ok(format!("Focused window {id}"))
                }
                Some("launch") => {
                    let cmd = parts[2..].join(" ");
                    if cmd.is_empty() {
                        return Err(anyhow::anyhow!("Usage: window launch <command>"));
                    }
                    let pid = self.aci.launch_app(&cmd)?;
                    Ok(format!("Launched {cmd:?} (pid={pid})"))
                }
                _ => Ok("Usage: window <list|focus|launch> [args]".into()),
            },
            Some(&"clipboard") => match parts.get(1).copied() {
                Some("get") => {
                    let content = self.aci.get_clipboard()?;
                    Ok(format!("Clipboard: {content:?}"))
                }
                Some("set") => {
                    let text = parts[2..].join(" ");
                    if text.is_empty() {
                        return Err(anyhow::anyhow!("Usage: clipboard set <text>"));
                    }
                    self.aci.set_clipboard(&text)?;
                    Ok(format!("Clipboard set to {text:?}"))
                }
                _ => Ok("Usage: clipboard <get|set> [text]".into()),
            },
            _ => Ok(
                "Usage: /computer <screenshot|tree|click|rclick|type|find|observe|ground|window|clipboard> [args]".into(),
            ),
        }
    }
}

fn format_tree(elem: &crate::r#impl::driver::types::Element, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut out = format!(
        "{indent}[{}] {:?} bounds={:?}\n",
        elem.role, elem.name, elem.bounds
    );
    for child in &elem.children {
        out += &format_tree(child, depth + 1);
    }
    out
}
