use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub app_name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub focused: bool,
}

pub trait WindowManager: Send + Sync {
    fn list_windows(&self) -> Result<Vec<WindowInfo>>;
    fn focus_window(&self, id: u64) -> Result<()>;
    fn launch_app(&self, command: &str) -> Result<u32>; // returns PID
}

pub struct MockWindowManager;

impl MockWindowManager {
    pub fn new() -> Self {
        Self
    }
}

impl WindowManager for MockWindowManager {
    fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        Ok(vec![WindowInfo {
            id: 1,
            title: "Mock Window".into(),
            app_name: "mock".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            focused: true,
        }])
    }

    fn focus_window(&self, _id: u64) -> Result<()> {
        Ok(())
    }

    fn launch_app(&self, _command: &str) -> Result<u32> {
        Ok(12345)
    }
}
