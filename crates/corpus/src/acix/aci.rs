use super::grounding::{GroundingProvider, GroundingResult};
use crate::drivers::{
    a11y::A11yDriver,
    display::{ClipboardDriver, DisplayDriver, WindowInfo, WindowManager},
    input::InputDriver,
    ocr::OcrDriver,
    types::*,
};
use anyhow::Result;

/// ACI — Agent-Computer Interface
///
/// Unified interface that composes input/display/a11y/ocr drivers,
/// providing high-level perception and action APIs.
pub struct Aci {
    input: Box<dyn InputDriver>,
    display: Box<dyn DisplayDriver>,
    a11y: Box<dyn A11yDriver>,
    ocr: Option<Box<dyn OcrDriver>>,
    window: Option<Box<dyn WindowManager>>,
    clipboard: Option<Box<dyn ClipboardDriver>>,
}

impl Aci {
    /// Create a new ACI instance
    pub fn new(
        input: Box<dyn InputDriver>,
        display: Box<dyn DisplayDriver>,
        a11y: Box<dyn A11yDriver>,
        ocr: Option<Box<dyn OcrDriver>>,
        window: Option<Box<dyn WindowManager>>,
        clipboard: Option<Box<dyn ClipboardDriver>>,
    ) -> Self {
        Self {
            input,
            display,
            a11y,
            ocr,
            window,
            clipboard,
        }
    }

    /// Create a new ACI instance with no window manager or clipboard support.
    pub fn new_basic(
        input: Box<dyn InputDriver>,
        display: Box<dyn DisplayDriver>,
        a11y: Box<dyn A11yDriver>,
        ocr: Option<Box<dyn OcrDriver>>,
    ) -> Self {
        Self::new(input, display, a11y, ocr, None, None)
    }

    // -- Perception --------------------------------------------------------

    /// Take a full-screen screenshot
    pub fn screenshot(&self) -> Result<Image> {
        self.display.screenshot()
    }

    /// Take a region screenshot
    pub fn screenshot_region(&self, x: i32, y: i32, w: i32, h: i32) -> Result<Image> {
        self.display.screenshot_region(x, y, w, h)
    }

    /// Get AT-SPI2 accessibility tree
    pub fn get_accessibility_tree(&self) -> Result<UiTree> {
        self.a11y.get_tree()
    }

    /// Get the element at screen coordinates
    pub fn get_element_at(&self, x: i32, y: i32) -> Result<Option<Element>> {
        self.a11y.get_element_at(x, y)
    }

    /// Find elements by description
    pub fn find_element(&self, description: &str) -> Result<Vec<Element>> {
        self.a11y.find_element(description)
    }

    // -- Actions -----------------------------------------------------------

    /// Left-click at (x, y)
    pub fn click(&self, x: i32, y: i32) -> Result<()> {
        self.input.click(x, y, MouseButton::Left)
    }

    /// Right-click at (x, y)
    pub fn right_click(&self, x: i32, y: i32) -> Result<()> {
        self.input.click(x, y, MouseButton::Right)
    }

    /// Type text
    pub fn type_text(&self, text: &str) -> Result<()> {
        self.input.type_text(text)
    }

    /// Press a key combination
    pub fn hotkey(&self, keys: &[Key]) -> Result<()> {
        self.input.hotkey(keys)
    }

    /// Scroll
    pub fn scroll(&self, x: i32, y: i32, direction: ScrollDirection, amount: i32) -> Result<()> {
        self.input.scroll(x, y, direction, amount)
    }

    /// Drag from (x1,y1) to (x2,y2)
    pub fn drag(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> Result<()> {
        self.input.drag(x1, y1, x2, y2)
    }

    // -- Window Management -------------------------------------------------

    /// List all open windows
    pub fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        self.window
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Window manager not available"))?
            .list_windows()
    }

    /// Focus a window by its ID
    pub fn focus_window(&self, id: u64) -> Result<()> {
        self.window
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Window manager not available"))?
            .focus_window(id)
    }

    /// Launch an application and return its PID
    pub fn launch_app(&self, command: &str) -> Result<u32> {
        self.window
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Window manager not available"))?
            .launch_app(command)
    }

    // -- Clipboard ---------------------------------------------------------

    /// Get clipboard content
    pub fn get_clipboard(&self) -> Result<String> {
        self.clipboard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Clipboard driver not available"))?
            .get_clipboard()
    }

    /// Set clipboard content
    pub fn set_clipboard(&self, text: &str) -> Result<()> {
        self.clipboard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Clipboard driver not available"))?
            .set_clipboard(text)
    }

    // -- Strategy ----------------------------------------------------------

    /// Smart observe: AT-SPI2 priority -> OCR fallback -> screenshot only
    pub fn smart_observe(&self) -> Result<Observation> {
        // 1. Try AT-SPI2
        match self.a11y.get_tree() {
            Ok(tree) if !tree.root.children.is_empty() => {
                return Ok(Observation::AccessibilityTree(tree));
            }
            _ => {}
        }

        // 2. Try OCR
        if let Some(ref ocr) = self.ocr {
            if let Ok(img) = self.display.screenshot() {
                match ocr.recognize(&img) {
                    Ok(result) if !result.text.is_empty() => {
                        return Ok(Observation::OcrFallback(result));
                    }
                    _ => {}
                }
            }
        }

        // 3. Screenshot only
        let img = self.display.screenshot()?;
        Ok(Observation::ScreenshotOnly(img))
    }

    // -- Grounding ---------------------------------------------------------

    /// Locate a UI element by natural language description using visual grounding.
    ///
    /// Takes a screenshot and sends it to the grounding provider (typically a
    /// vision-capable LLM) to find the element matching the description.
    pub async fn locate_element(
        &self,
        description: &str,
        grounding: &dyn GroundingProvider,
    ) -> Result<GroundingResult> {
        let img = self.screenshot()?;
        grounding.locate(&img, description).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::{
        a11y::MockA11yDriver,
        display::{MockClipboardDriver, MockDisplayDriver, MockWindowManager},
        input::MockInputDriver,
        ocr::MockOcrDriver,
    };

    fn mock_aci() -> Aci {
        Aci::new(
            Box::new(MockInputDriver::new()),
            Box::new(MockDisplayDriver::new(1920, 1080)),
            Box::new(MockA11yDriver::new()),
            Some(Box::new(MockOcrDriver)),
            Some(Box::new(MockWindowManager::new())),
            Some(Box::new(MockClipboardDriver::new())),
        )
    }

    #[test]
    fn test_screenshot() {
        let aci = mock_aci();
        let img = aci.screenshot().unwrap();
        assert_eq!(img.width, 1920);
    }

    #[test]
    fn test_click() {
        let aci = mock_aci();
        aci.click(100, 200).unwrap();
        // MockInputDriver logs it
    }

    #[test]
    fn test_get_tree() {
        let aci = mock_aci();
        let tree = aci.get_accessibility_tree().unwrap();
        assert_eq!(tree.app_name, "MockApp");
    }

    #[test]
    fn test_find_element() {
        let aci = mock_aci();
        let elems = aci.find_element("ok").unwrap();
        assert_eq!(elems.len(), 1);
        assert_eq!(elems[0].name, "OK");
    }

    #[test]
    fn test_smart_observe_atspi() {
        let aci = mock_aci();
        let obs = aci.smart_observe().unwrap();
        match obs {
            Observation::AccessibilityTree(tree) => {
                assert_eq!(tree.app_name, "MockApp");
            }
            _ => panic!("Expected AccessibilityTree"),
        }
    }

    #[test]
    fn test_list_windows() {
        let aci = mock_aci();
        let wins = aci.list_windows().unwrap();
        assert_eq!(wins.len(), 1);
        assert_eq!(wins[0].title, "Mock Window");
    }

    #[test]
    fn test_focus_window() {
        let aci = mock_aci();
        aci.focus_window(1).unwrap();
    }

    #[test]
    fn test_launch_app() {
        let aci = mock_aci();
        let pid = aci.launch_app("firefox").unwrap();
        assert_eq!(pid, 12345);
    }

    #[test]
    fn test_clipboard_roundtrip() {
        let aci = mock_aci();
        aci.set_clipboard("hello").unwrap();
        assert_eq!(aci.get_clipboard().unwrap(), "hello");
    }

    #[test]
    fn test_window_manager_missing() {
        let aci = Aci::new_basic(
            Box::new(MockInputDriver::new()),
            Box::new(MockDisplayDriver::new(1920, 1080)),
            Box::new(MockA11yDriver::new()),
            None,
        );
        assert!(aci.list_windows().is_err());
    }

    #[test]
    fn test_clipboard_missing() {
        let aci = Aci::new_basic(
            Box::new(MockInputDriver::new()),
            Box::new(MockDisplayDriver::new(1920, 1080)),
            Box::new(MockA11yDriver::new()),
            None,
        );
        assert!(aci.get_clipboard().is_err());
    }
}
