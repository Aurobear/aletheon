//! ACIX tool registrations for the Engine's ToolRegistry.
//!
//! Each tool wraps an `Arc<Aci>` and delegates to the unified
//! Agent-Computer Interface. Register all tools at once via
//! `register_acix_tools`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::acix::Aci;
use crate::acix::GroundingProvider;
use corpus::drivers::types::{Key, ScrollDirection};
use corpus::tools::tools::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use fabric::Registry;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn ok(content: String, elapsed_ms: u64) -> ToolResult {
    ToolResult {
        content,
        is_error: false,
        metadata: ToolResultMeta {
            execution_time_ms: elapsed_ms,
            truncated: false,
        },
    }
}

fn err(msg: impl Into<String>, elapsed_ms: u64) -> ToolResult {
    ToolResult {
        content: msg.into(),
        is_error: true,
        metadata: ToolResultMeta {
            execution_time_ms: elapsed_ms,
            truncated: false,
        },
    }
}

// ---------------------------------------------------------------------------
// ScreenshotTool
// ---------------------------------------------------------------------------

pub struct ScreenshotTool {
    aci: Arc<Aci>,
}

#[async_trait]
impl Tool for ScreenshotTool {
    fn name(&self) -> &str {
        "acix_screenshot"
    }

    fn description(&self) -> &str {
        "Take a full-screen screenshot. Returns image dimensions and byte size."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ScreenshotTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        match self.aci.screenshot() {
            Ok(img) => ok(
                format!(
                    "{}x{} ({} bytes RGB)",
                    img.width,
                    img.height,
                    img.data.len()
                ),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// TreeTool
// ---------------------------------------------------------------------------

pub struct TreeTool {
    aci: Arc<Aci>,
}

#[async_trait]
impl Tool for TreeTool {
    fn name(&self) -> &str {
        "acix_tree"
    }

    fn description(&self) -> &str {
        "Get the AT-SPI2 accessibility tree of the focused application. Returns structured UI element hierarchy."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(TreeTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        match self.aci.get_accessibility_tree() {
            Ok(tree) => {
                let formatted = format_element(&tree.root, 0);
                ok(
                    format!("app: {}\n{}", tree.app_name, formatted),
                    start.elapsed().as_millis() as u64,
                )
            }
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

fn format_element(elem: &corpus::drivers::types::Element, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut out = format!(
        "{}[{}] {:?} bounds=({},{}+{}x{})\n",
        indent,
        elem.role,
        elem.name,
        elem.bounds.x,
        elem.bounds.y,
        elem.bounds.width,
        elem.bounds.height,
    );
    for child in &elem.children {
        out += &format_element(child, depth + 1);
    }
    out
}

// ---------------------------------------------------------------------------
// ClickTool
// ---------------------------------------------------------------------------

pub struct ClickTool {
    aci: Arc<Aci>,
}

#[async_trait]
impl Tool for ClickTool {
    fn name(&self) -> &str {
        "acix_click"
    }

    fn description(&self) -> &str {
        "Left-click at screen coordinates (x, y)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "Screen x coordinate" },
                "y": { "type": "integer", "description": "Screen y coordinate" }
            },
            "required": ["x", "y"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ClickTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let x = input["x"].as_i64().unwrap_or(0) as i32;
        let y = input["y"].as_i64().unwrap_or(0) as i32;
        match self.aci.click(x, y) {
            Ok(()) => ok(
                format!("Clicked at ({}, {})", x, y),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// TypeTool
// ---------------------------------------------------------------------------

pub struct TypeTool {
    aci: Arc<Aci>,
}

#[async_trait]
impl Tool for TypeTool {
    fn name(&self) -> &str {
        "acix_type"
    }

    fn description(&self) -> &str {
        "Type text via virtual keyboard input."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to type" }
            },
            "required": ["text"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(TypeTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let text = input["text"].as_str().unwrap_or("");
        match self.aci.type_text(text) {
            Ok(()) => ok(
                format!("Typed: {:?}", text),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// FindTool
// ---------------------------------------------------------------------------

pub struct FindTool {
    aci: Arc<Aci>,
}

#[async_trait]
impl Tool for FindTool {
    fn name(&self) -> &str {
        "acix_find"
    }

    fn description(&self) -> &str {
        "Find UI elements by description. Searches element name, role, and text content."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Keyword to search for in element names, roles, or text"
                }
            },
            "required": ["description"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(FindTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let desc = input["description"].as_str().unwrap_or("");
        match self.aci.find_element(desc) {
            Ok(elems) => {
                if elems.is_empty() {
                    ok(
                        "No elements found.".to_string(),
                        start.elapsed().as_millis() as u64,
                    )
                } else {
                    let mut out = format!("Found {} element(s):\n", elems.len());
                    for (i, e) in elems.iter().enumerate() {
                        out += &format!(
                            "  [{}] role={} name={:?} bounds=({},{}+{}x{})\n",
                            i,
                            e.role,
                            e.name,
                            e.bounds.x,
                            e.bounds.y,
                            e.bounds.width,
                            e.bounds.height,
                        );
                    }
                    ok(out, start.elapsed().as_millis() as u64)
                }
            }
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// ObserveTool
// ---------------------------------------------------------------------------

pub struct ObserveTool {
    aci: Arc<Aci>,
}

#[async_trait]
impl Tool for ObserveTool {
    fn name(&self) -> &str {
        "acix_observe"
    }

    fn description(&self) -> &str {
        "Smart observation: tries AT-SPI2 accessibility tree first, falls back to OCR, then screenshot only."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ObserveTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        match self.aci.smart_observe() {
            Ok(observation) => {
                let text = match observation {
                    corpus::drivers::types::Observation::AccessibilityTree(tree) => {
                        format!(
                            "[AT-SPI2] app: {}\n{}",
                            tree.app_name,
                            format_element(&tree.root, 0),
                        )
                    }
                    corpus::drivers::types::Observation::OcrFallback(ocr) => {
                        let words: Vec<&str> = ocr.words.iter().map(|w| w.text.as_str()).collect();
                        format!("[OCR] text: {}\nwords: {}", ocr.text, words.join(", "))
                    }
                    corpus::drivers::types::Observation::ScreenshotOnly(img) => {
                        format!(
                            "[Screenshot] {}x{} ({} bytes)",
                            img.width,
                            img.height,
                            img.data.len()
                        )
                    }
                };
                ok(text, start.elapsed().as_millis() as u64)
            }
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// DragTool
// ---------------------------------------------------------------------------

pub struct DragTool {
    aci: Arc<Aci>,
}

#[async_trait]
impl Tool for DragTool {
    fn name(&self) -> &str {
        "acix_drag"
    }

    fn description(&self) -> &str {
        "Drag from (x1, y1) to (x2, y2) on screen."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "x1": { "type": "integer", "description": "Start x coordinate" },
                "y1": { "type": "integer", "description": "Start y coordinate" },
                "x2": { "type": "integer", "description": "End x coordinate" },
                "y2": { "type": "integer", "description": "End y coordinate" }
            },
            "required": ["x1", "y1", "x2", "y2"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(DragTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let x1 = input["x1"].as_i64().unwrap_or(0) as i32;
        let y1 = input["y1"].as_i64().unwrap_or(0) as i32;
        let x2 = input["x2"].as_i64().unwrap_or(0) as i32;
        let y2 = input["y2"].as_i64().unwrap_or(0) as i32;
        match self.aci.drag(x1, y1, x2, y2) {
            Ok(()) => ok(
                format!("Dragged from ({}, {}) to ({}, {})", x1, y1, x2, y2),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// HotkeyTool
// ---------------------------------------------------------------------------

pub struct HotkeyTool {
    aci: Arc<Aci>,
}

fn parse_key(s: &str) -> Option<Key> {
    match s.to_lowercase().as_str() {
        "a" => Some(Key::A),
        "b" => Some(Key::B),
        "c" => Some(Key::C),
        "d" => Some(Key::D),
        "e" => Some(Key::E),
        "f" => Some(Key::F),
        "g" => Some(Key::G),
        "h" => Some(Key::H),
        "i" => Some(Key::I),
        "j" => Some(Key::J),
        "k" => Some(Key::K),
        "l" => Some(Key::L),
        "m" => Some(Key::M),
        "n" => Some(Key::N),
        "o" => Some(Key::O),
        "p" => Some(Key::P),
        "q" => Some(Key::Q),
        "r" => Some(Key::R),
        "s" => Some(Key::S),
        "t" => Some(Key::T),
        "u" => Some(Key::U),
        "v" => Some(Key::V),
        "w" => Some(Key::W),
        "x" => Some(Key::X),
        "y" => Some(Key::Y),
        "z" => Some(Key::Z),
        "0" => Some(Key::Num0),
        "1" => Some(Key::Num1),
        "2" => Some(Key::Num2),
        "3" => Some(Key::Num3),
        "4" => Some(Key::Num4),
        "5" => Some(Key::Num5),
        "6" => Some(Key::Num6),
        "7" => Some(Key::Num7),
        "8" => Some(Key::Num8),
        "9" => Some(Key::Num9),
        "enter" | "return" => Some(Key::Enter),
        "space" => Some(Key::Space),
        "tab" => Some(Key::Tab),
        "escape" | "esc" => Some(Key::Escape),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" | "page_up" => Some(Key::PageUp),
        "pagedown" | "page_down" => Some(Key::PageDown),
        "f1" => Some(Key::F1),
        "f2" => Some(Key::F2),
        "f3" => Some(Key::F3),
        "f4" => Some(Key::F4),
        "f5" => Some(Key::F5),
        "f6" => Some(Key::F6),
        "f7" => Some(Key::F7),
        "f8" => Some(Key::F8),
        "f9" => Some(Key::F9),
        "f10" => Some(Key::F10),
        "f11" => Some(Key::F11),
        "f12" => Some(Key::F12),
        "ctrl" | "control" => Some(Key::Ctrl),
        "alt" => Some(Key::Alt),
        "shift" => Some(Key::Shift),
        "super" | "meta" | "win" => Some(Key::Super),
        _ => None,
    }
}

#[async_trait]
impl Tool for HotkeyTool {
    fn name(&self) -> &str {
        "acix_hotkey"
    }

    fn description(&self) -> &str {
        "Press a key combination (e.g. [\"ctrl\", \"c\"]). Keys: a-z, 0-9, enter, space, tab, escape, backspace, delete, arrow keys, home, end, pageup, pagedown, f1-f12, ctrl, alt, shift, super."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Key names to press simultaneously (e.g. [\"ctrl\", \"c\"])"
                }
            },
            "required": ["keys"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(HotkeyTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let keys_input: Vec<String> = input["keys"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let mut parsed = Vec::new();
        for k in &keys_input {
            match parse_key(k) {
                Some(key) => parsed.push(key),
                None => {
                    return err(
                        format!("Unknown key: {:?}", k),
                        start.elapsed().as_millis() as u64,
                    )
                }
            }
        }

        match self.aci.hotkey(&parsed) {
            Ok(()) => ok(
                format!("Pressed: {}", keys_input.join(" + ")),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// ScrollTool
// ---------------------------------------------------------------------------

pub struct ScrollTool {
    aci: Arc<Aci>,
}

fn parse_scroll_direction(s: &str) -> Option<ScrollDirection> {
    match s.to_lowercase().as_str() {
        "up" => Some(ScrollDirection::Up),
        "down" => Some(ScrollDirection::Down),
        "left" => Some(ScrollDirection::Left),
        "right" => Some(ScrollDirection::Right),
        _ => None,
    }
}

#[async_trait]
impl Tool for ScrollTool {
    fn name(&self) -> &str {
        "acix_scroll"
    }

    fn description(&self) -> &str {
        "Scroll at screen coordinates (x, y) in the given direction."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "Screen x coordinate" },
                "y": { "type": "integer", "description": "Screen y coordinate" },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "Scroll direction"
                },
                "amount": {
                    "type": "integer",
                    "description": "Scroll amount (number of notches)",
                    "default": 3
                }
            },
            "required": ["x", "y", "direction"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ScrollTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let x = input["x"].as_i64().unwrap_or(0) as i32;
        let y = input["y"].as_i64().unwrap_or(0) as i32;
        let dir_str = input["direction"].as_str().unwrap_or("down");
        let amount = input["amount"].as_i64().unwrap_or(3) as i32;

        let direction = match parse_scroll_direction(dir_str) {
            Some(d) => d,
            None => {
                return err(
                    format!("Unknown scroll direction: {:?}", dir_str),
                    start.elapsed().as_millis() as u64,
                )
            }
        };

        match self.aci.scroll(x, y, direction, amount) {
            Ok(()) => ok(
                format!("Scrolled {} x{} at ({}, {})", dir_str, amount, x, y),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// RightClickTool
// ---------------------------------------------------------------------------

pub struct RightClickTool {
    aci: Arc<Aci>,
}

#[async_trait]
impl Tool for RightClickTool {
    fn name(&self) -> &str {
        "acix_rclick"
    }

    fn description(&self) -> &str {
        "Right-click at screen coordinates (x, y)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "Screen x coordinate" },
                "y": { "type": "integer", "description": "Screen y coordinate" }
            },
            "required": ["x", "y"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(RightClickTool {
            aci: Arc::clone(&self.aci),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let x = input["x"].as_i64().unwrap_or(0) as i32;
        let y = input["y"].as_i64().unwrap_or(0) as i32;
        match self.aci.right_click(x, y) {
            Ok(()) => ok(
                format!("Right-clicked at ({}, {})", x, y),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// AcixGroundTool
// ---------------------------------------------------------------------------

pub struct AcixGroundTool {
    aci: Arc<Aci>,
    grounding: Arc<dyn GroundingProvider>,
}

impl AcixGroundTool {
    pub fn new(aci: Arc<Aci>, grounding: Arc<dyn GroundingProvider>) -> Self {
        Self { aci, grounding }
    }
}

#[async_trait]
impl Tool for AcixGroundTool {
    fn name(&self) -> &str {
        "acix_ground"
    }

    fn description(&self) -> &str {
        "Locate a UI element on screen by natural language description. Returns x, y coordinates and bounding box."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Natural language description of the UI element to locate"
                }
            },
            "required": ["description"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(AcixGroundTool {
            aci: Arc::clone(&self.aci),
            grounding: Arc::clone(&self.grounding),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let desc = input["description"].as_str().unwrap_or("");

        let image = match self.aci.screenshot() {
            Ok(img) => img,
            Err(e) => return err(e.to_string(), start.elapsed().as_millis() as u64),
        };

        match self.grounding.locate(&image, desc).await {
            Ok(result) => ok(
                serde_json::to_string(&serde_json::json!({
                    "x": result.x,
                    "y": result.y,
                    "width": result.width,
                    "height": result.height,
                    "confidence": result.confidence,
                    "label": result.label,
                }))
                .unwrap_or_default(),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => err(e.to_string(), start.elapsed().as_millis() as u64),
        }
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Create a default `Aci` instance using mock drivers.
///
/// This is a convenience for testing and CLI usage. Production code should
/// construct `Aci` with real driver implementations and call
/// `register_acix_tools_with` instead.
///
/// # Panics
///
/// Panics if called without the `input`, `display`, and `a11y` features enabled on the `drivers` crate.
pub fn default_aci() -> Arc<Aci> {
    Arc::new(Aci::new_basic(
        Box::new(corpus::drivers::input::MockInputDriver::new()),
        Box::new(corpus::drivers::display::MockDisplayDriver::new(1920, 1080)),
        Box::new(corpus::drivers::a11y::MockA11yDriver::new()),
        Some(Box::new(corpus::drivers::ocr::MockOcrDriver)),
    ))
}

/// Register all ACIX tools into the given `ToolRegistry` using a caller-provided `Aci`.
pub fn register_acix_tools_with(registry: &mut corpus::tools::tools::ToolRegistry, aci: Arc<Aci>) {
    let _ = registry.register(Arc::new(ScreenshotTool {
        aci: Arc::clone(&aci),
    }));
    let _ = registry.register(Arc::new(TreeTool {
        aci: Arc::clone(&aci),
    }));
    let _ = registry.register(Arc::new(ClickTool {
        aci: Arc::clone(&aci),
    }));
    let _ = registry.register(Arc::new(TypeTool {
        aci: Arc::clone(&aci),
    }));
    let _ = registry.register(Arc::new(FindTool {
        aci: Arc::clone(&aci),
    }));
    let _ = registry.register(Arc::new(ObserveTool {
        aci: Arc::clone(&aci),
    }));
    let _ = registry.register(Arc::new(DragTool {
        aci: Arc::clone(&aci),
    }));
    let _ = registry.register(Arc::new(HotkeyTool {
        aci: Arc::clone(&aci),
    }));
    let _ = registry.register(Arc::new(ScrollTool {
        aci: Arc::clone(&aci),
    }));
    let _ = registry.register(Arc::new(RightClickTool { aci }));
}

/// Register all ACIX tools into the given `ToolRegistry` with mock drivers.
///
/// Convenience for testing. For production, use `register_acix_tools_with`.
pub fn register_acix_tools(registry: &mut corpus::tools::tools::ToolRegistry) {
    register_acix_tools_with(registry, default_aci());
}

/// Register the `acix_ground` tool into the given `ToolRegistry`.
///
/// Requires a caller-provided `Aci` and `GroundingProvider`. The grounding
/// provider is typically a `VisionGroundingProvider` backed by a multimodal LLM.
pub fn register_acix_ground_tool(
    registry: &mut corpus::tools::tools::ToolRegistry,
    aci: Arc<Aci>,
    grounding: Arc<dyn GroundingProvider>,
) {
    let _ = registry.register(Arc::new(AcixGroundTool::new(aci, grounding)));
}
