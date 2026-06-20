use crate::r#impl::driver::types::{Key, MouseButton, ScrollDirection};
use anyhow::Result;

/// Input device driver trait
pub trait InputDriver: Send + Sync {
    /// Mouse click
    fn click(&self, x: i32, y: i32, button: MouseButton) -> Result<()>;
    /// Type text via keyboard
    fn type_text(&self, text: &str) -> Result<()>;
    /// Hotkey combination
    fn hotkey(&self, keys: &[Key]) -> Result<()>;
    /// Scroll wheel
    fn scroll(&self, x: i32, y: i32, direction: ScrollDirection, amount: i32) -> Result<()>;
    /// Drag from (x1,y1) to (x2,y2)
    fn drag(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> Result<()>;
}

/// Mock input driver for testing
pub struct MockInputDriver {
    pub log: std::sync::Mutex<Vec<String>>,
}

impl MockInputDriver {
    pub fn new() -> Self {
        Self {
            log: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl InputDriver for MockInputDriver {
    fn click(&self, x: i32, y: i32, button: MouseButton) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("click({x},{y},{button:?})"));
        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<()> {
        self.log.lock().unwrap().push(format!("type({text:?})"));
        Ok(())
    }

    fn hotkey(&self, keys: &[Key]) -> Result<()> {
        let key_strs: Vec<String> = keys.iter().map(|k| format!("{k:?}")).collect();
        self.log
            .lock()
            .unwrap()
            .push(format!("hotkey({})", key_strs.join("+")));
        Ok(())
    }

    fn scroll(&self, x: i32, y: i32, direction: ScrollDirection, amount: i32) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("scroll({x},{y},{direction:?},{amount})"));
        Ok(())
    }

    fn drag(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> Result<()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("drag({x1},{y1},{x2},{y2})"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_click() {
        let driver = MockInputDriver::new();
        driver.click(100, 200, MouseButton::Left).unwrap();
        let log = driver.log.lock().unwrap();
        assert_eq!(log[0], "click(100,200,Left)");
    }

    #[test]
    fn test_mock_type() {
        let driver = MockInputDriver::new();
        driver.type_text("hello").unwrap();
        let log = driver.log.lock().unwrap();
        assert_eq!(log[0], "type(\"hello\")");
    }

    #[test]
    fn test_mock_hotkey() {
        let driver = MockInputDriver::new();
        driver.hotkey(&[Key::Ctrl, Key::C]).unwrap();
        let log = driver.log.lock().unwrap();
        assert_eq!(log[0], "hotkey(Ctrl+C)");
    }

    #[test]
    fn test_mock_scroll() {
        let driver = MockInputDriver::new();
        driver.scroll(500, 300, ScrollDirection::Down, 3).unwrap();
        let log = driver.log.lock().unwrap();
        assert_eq!(log[0], "scroll(500,300,Down,3)");
    }

    #[test]
    fn test_mock_drag() {
        let driver = MockInputDriver::new();
        driver.drag(10, 20, 300, 400).unwrap();
        let log = driver.log.lock().unwrap();
        assert_eq!(log[0], "drag(10,20,300,400)");
    }

    #[test]
    fn test_mock_multiple_actions() {
        let driver = MockInputDriver::new();
        driver.click(0, 0, MouseButton::Right).unwrap();
        driver.type_text("world").unwrap();
        driver.hotkey(&[Key::Alt, Key::Tab]).unwrap();
        let log = driver.log.lock().unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0], "click(0,0,Right)");
        assert_eq!(log[1], "type(\"world\")");
        assert_eq!(log[2], "hotkey(Alt+Tab)");
    }
}

#[cfg(target_os = "linux")]
pub mod uinput;
#[cfg(target_os = "linux")]
pub use uinput::UinputDriver;
