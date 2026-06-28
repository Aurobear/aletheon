use anyhow::Result;

pub trait ClipboardDriver: Send + Sync {
    fn get_clipboard(&self) -> Result<String>;
    fn set_clipboard(&self, text: &str) -> Result<()>;
}

pub struct MockClipboardDriver {
    content: std::sync::Mutex<String>,
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
        Ok(self.content.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    fn set_clipboard(&self, text: &str) -> Result<()> {
        *self.content.lock().unwrap_or_else(|e| e.into_inner()) = text.to_string();
        Ok(())
    }
}
