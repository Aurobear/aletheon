use crate::r#impl::driver::types::Image;
use anyhow::Result;

/// Display driver trait
pub trait DisplayDriver: Send + Sync {
    /// Full-screen screenshot
    fn screenshot(&self) -> Result<Image>;
    /// Region screenshot
    fn screenshot_region(&self, x: i32, y: i32, w: i32, h: i32) -> Result<Image>;
}

/// Mock display driver for testing
pub struct MockDisplayDriver {
    pub width: u32,
    pub height: u32,
}

impl MockDisplayDriver {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl DisplayDriver for MockDisplayDriver {
    fn screenshot(&self) -> Result<Image> {
        Ok(Image {
            width: self.width,
            height: self.height,
            data: vec![0u8; (self.width * self.height * 3) as usize],
        })
    }

    fn screenshot_region(&self, x: i32, y: i32, w: i32, h: i32) -> Result<Image> {
        let _ = (x, y); // coordinates unused in mock
        let w = w.max(0) as u32;
        let h = h.max(0) as u32;
        Ok(Image {
            width: w,
            height: h,
            data: vec![0u8; (w * h * 3) as usize],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_screenshot() {
        let driver = MockDisplayDriver::new(1920, 1080);
        let img = driver.screenshot().unwrap();
        assert_eq!(img.width, 1920);
        assert_eq!(img.height, 1080);
        assert_eq!(img.data.len(), 1920 * 1080 * 3);
    }

    #[test]
    fn test_mock_screenshot_region() {
        let driver = MockDisplayDriver::new(1920, 1080);
        let img = driver.screenshot_region(100, 200, 640, 480).unwrap();
        assert_eq!(img.width, 640);
        assert_eq!(img.height, 480);
        assert_eq!(img.data.len(), 640 * 480 * 3);
    }

    #[test]
    fn test_mock_screenshot_region_clamped() {
        let driver = MockDisplayDriver::new(1920, 1080);
        let img = driver.screenshot_region(0, 0, -10, -5).unwrap();
        assert_eq!(img.width, 0);
        assert_eq!(img.height, 0);
        assert_eq!(img.data.len(), 0);
    }
}

#[cfg(feature = "display")]
pub mod x11;

#[cfg(feature = "display")]
pub use x11::X11DisplayDriver;

pub mod clipboard;
pub mod window;

#[cfg(feature = "display")]
pub mod clipboard_x11;

#[cfg(feature = "display")]
pub use clipboard_x11::X11ClipboardDriver;

#[cfg(feature = "display")]
pub mod window_x11;

pub use clipboard::{ClipboardDriver, MockClipboardDriver};
pub use window::{MockWindowManager, WindowInfo, WindowManager};

#[cfg(feature = "display")]
pub use window_x11::EwmhWindowManager;

pub mod drm;
pub use drm::FramebufferDriver;
