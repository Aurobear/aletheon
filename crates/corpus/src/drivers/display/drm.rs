use super::DisplayDriver;
use crate::drivers::types::Image;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Framebuffer screenshot driver for headless systems.
///
/// Reads `/dev/fb0` directly to capture the screen without X11.
/// Works on systems with a kernel framebuffer (headless servers,
/// console-mode displays, some Wayland compositors).
pub struct FramebufferDriver {
    device_path: String,
}

impl FramebufferDriver {
    pub fn new() -> Result<Self> {
        let path = if std::path::Path::new("/dev/fb0").exists() {
            "/dev/fb0".to_string()
        } else {
            anyhow::bail!("No framebuffer device found at /dev/fb0");
        };

        // Verify we can open it
        let _file = File::open(&path).context("Failed to open framebuffer device")?;

        Ok(Self { device_path: path })
    }

    fn get_resolution(&self) -> Result<(u32, u32, u32)> {
        let size_path = "/sys/class/graphics/fb0/virtual_size";
        let bpp_path = "/sys/class/graphics/fb0/bits_per_pixel";

        let size_str =
            std::fs::read_to_string(size_path).context("Failed to read fb0 virtual_size")?;
        let parts: Vec<&str> = size_str.trim().split(',').collect();
        let width: u32 = parts[0].parse().context("Invalid width in virtual_size")?;
        let height: u32 = parts[1].parse().context("Invalid height in virtual_size")?;

        let bpp_str = std::fs::read_to_string(bpp_path).unwrap_or_else(|_| "32".to_string());
        let bpp: u32 = bpp_str.trim().parse().unwrap_or(32);

        Ok((width, height, bpp))
    }
}

impl DisplayDriver for FramebufferDriver {
    fn screenshot(&self) -> Result<Image> {
        let (width, height, bpp) = self.get_resolution()?;

        let mut file = File::open(&self.device_path).context("Failed to open framebuffer")?;

        let bytes_per_pixel = (bpp / 8) as usize;
        let total_bytes = (width * height) as usize * bytes_per_pixel;
        let mut buf = vec![0u8; total_bytes];

        file.read_exact(&mut buf)
            .context("Failed to read framebuffer data")?;

        // Convert to RGB (framebuffer is typically BGRA/BGRX)
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for chunk in buf.chunks(bytes_per_pixel) {
            if chunk.len() >= 3 {
                rgb.push(chunk[2]); // R
                rgb.push(chunk[1]); // G
                rgb.push(chunk[0]); // B
            }
        }

        Ok(Image {
            width,
            height,
            data: rgb,
        })
    }

    fn screenshot_region(&self, x: i32, y: i32, w: i32, h: i32) -> Result<Image> {
        let (full_w, full_h, bpp) = self.get_resolution()?;

        // Clamp region to screen bounds
        let x = x.max(0) as u32;
        let y = y.max(0) as u32;
        let w = (w.max(1) as u32).min(full_w.saturating_sub(x));
        let h = (h.max(1) as u32).min(full_h.saturating_sub(y));

        let mut file = File::open(&self.device_path)?;
        let bytes_per_pixel = (bpp / 8) as usize;
        let stride = full_w as usize * bytes_per_pixel;

        let mut rgb = Vec::with_capacity((w * h * 3) as usize);

        // Read line by line for the region
        for row in y..y + h {
            let offset = (row as usize * stride) + (x as usize * bytes_per_pixel);
            let mut line_buf = vec![0u8; w as usize * bytes_per_pixel];

            file.seek(SeekFrom::Start(offset as u64))?;
            file.read_exact(&mut line_buf)?;

            for chunk in line_buf.chunks(bytes_per_pixel) {
                if chunk.len() >= 3 {
                    rgb.push(chunk[2]);
                    rgb.push(chunk[1]);
                    rgb.push(chunk[0]);
                }
            }
        }

        Ok(Image {
            width: w,
            height: h,
            data: rgb,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_framebuffer_driver_creation() {
        // Will succeed only if /dev/fb0 exists; skip gracefully otherwise
        match FramebufferDriver::new() {
            Ok(driver) => {
                assert!(!driver.device_path.is_empty());
            }
            Err(_) => {
                // /dev/fb0 not available in this environment — skip
            }
        }
    }
}
