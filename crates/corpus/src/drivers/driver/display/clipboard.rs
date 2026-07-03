use anyhow::Result;

pub trait ClipboardDriver: Send + Sync {
    fn get_clipboard(&self) -> Result<String>;
    fn set_clipboard(&self, text: &str) -> Result<()>;
}

pub struct MockClipboardDriver {
    content: std::sync::Mutex<String>,
}

impl Default for MockClipboardDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl MockClipboardDriver {
    pub fn new() -> Self {
        Self {
            content: std::sync::Mutex::new(String::new()),
        }
    }
}

impl ClipboardDriver for MockClipboardDriver {
    fn get_clipboard(&self) -> Result<String> {
        Ok(self.content.lock().unwrap().clone())
    }

    fn set_clipboard(&self, text: &str) -> Result<()> {
        *self.content.lock().unwrap() = text.to_string();
        Ok(())
    }
}
