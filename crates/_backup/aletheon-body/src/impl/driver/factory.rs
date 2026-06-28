/// Detect available drivers and create real implementations.
pub struct DriverFactory;

impl DriverFactory {
    /// Try to create a real input driver (uinput on Linux).
    #[cfg(feature = "input")]
    pub fn try_input() -> Option<Box<dyn crate::r#impl::driver::input::InputDriver>> {
        #[cfg(target_os = "linux")]
        {
            if std::path::Path::new("/dev/uinput").exists() {
                match crate::r#impl::driver::input::UinputDriver::create() {
                    Ok(d) => return Some(Box::new(d)),
                    Err(e) => tracing::warn!("UinputDriver failed: {e}"),
                }
            }
        }
        None
    }

    /// Try to create a real display driver (X11, then framebuffer fallback).
    #[cfg(feature = "display")]
    pub fn try_display() -> Option<Box<dyn crate::r#impl::driver::display::DisplayDriver>> {
        match crate::r#impl::driver::display::X11DisplayDriver::new() {
            Ok(d) => return Some(Box::new(d)),
            Err(e) => tracing::warn!("X11DisplayDriver failed: {e}"),
        }
        // Fallback to framebuffer for headless systems
        match crate::r#impl::driver::display::FramebufferDriver::new() {
            Ok(d) => return Some(Box::new(d)),
            Err(e) => tracing::debug!("FramebufferDriver not available: {e}"),
        }
        None
    }

    /// Try to create a real a11y driver (AT-SPI2 via D-Bus).
    #[cfg(feature = "a11y")]
    pub fn try_a11y() -> Option<Box<dyn crate::r#impl::driver::a11y::A11yDriver>> {
        // AT-SPI2 requires a D-Bus session bus
        if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok() {
            match crate::r#impl::driver::a11y::AtSpiDriver::new() {
                Ok(d) => return Some(Box::new(d)),
                Err(e) => tracing::warn!("AtSpiDriver failed: {e}"),
            }
        }
        None
    }

    /// Try to create a real OCR driver (Tesseract if available).
    #[cfg(feature = "ocr-tesseract")]
    pub fn try_ocr() -> Option<Box<dyn crate::r#impl::driver::ocr::OcrDriver>> {
        match crate::r#impl::driver::ocr::tesseract::TesseractOcrDriver::new() {
            Ok(d) => return Some(Box::new(d)),
            Err(e) => tracing::warn!("TesseractOcrDriver failed: {e}"),
        }
    }

    /// Try to create a real window manager (EWMH via X11).
    #[cfg(feature = "display")]
    pub fn try_window() -> Option<Box<dyn crate::r#impl::driver::display::WindowManager>> {
        // Check for DISPLAY (X11) or WAYLAND_DISPLAY
        if std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok() {
            return Some(Box::new(crate::r#impl::driver::display::EwmhWindowManager::new()));
        }
        None
    }

    /// Try to create a real clipboard driver (X11 clipboard).
    #[cfg(feature = "display")]
    pub fn try_clipboard() -> Option<Box<dyn crate::r#impl::driver::display::ClipboardDriver>> {
        if std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok() {
            return Some(Box::new(crate::r#impl::driver::display::X11ClipboardDriver::new()));
        }
        None
    }
}
