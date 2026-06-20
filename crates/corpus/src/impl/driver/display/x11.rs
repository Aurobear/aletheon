use anyhow::{Context, Result};
use x11rb::connection::Connection;

use super::DisplayDriver;
use crate::r#impl::driver::types::Image;

/// X11 screenshot driver using XGetImage
pub struct X11DisplayDriver {
    // Connection is created per-call to avoid Send issues
}

impl X11DisplayDriver {
    pub fn new() -> Result<Self> {
        // Verify X11 is available
        x11rb::connect(None).context("Failed to connect to X11 display")?;
        Ok(Self {})
    }
}

impl DisplayDriver for X11DisplayDriver {
    fn screenshot(&self) -> Result<Image> {
        use x11rb::protocol::xproto::*;

        let (conn, screen_num) = x11rb::connect(None).context("Failed to connect to X11")?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let width = screen.width_in_pixels as u32;
        let height = screen.height_in_pixels as u32;

        let reply = get_image(
            &conn,
            ImageFormat::Z_PIXMAP,
            root,
            0,
            0,
            width as u16,
            height as u16,
            !0u32, // plane_mask: all planes
        )?
        .reply()
        .context("XGetImage failed")?;

        // reply.data is BGRA, convert to RGB
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for chunk in reply.data.chunks(4) {
            if chunk.len() >= 3 {
                rgb.push(chunk[2]); // R (BGRA -> RGB)
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
        use x11rb::protocol::xproto::*;

        let (conn, screen_num) = x11rb::connect(None).context("Failed to connect to X11")?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

        let w = w.max(1) as u16;
        let h = h.max(1) as u16;

        let reply = get_image(
            &conn,
            ImageFormat::Z_PIXMAP,
            root,
            x.max(0) as i16,
            y.max(0) as i16,
            w,
            h,
            !0u32,
        )?
        .reply()
        .context("XGetImage failed")?;

        let mut rgb = Vec::with_capacity((w as u32 * h as u32 * 3) as usize);
        for chunk in reply.data.chunks(4) {
            if chunk.len() >= 3 {
                rgb.push(chunk[2]);
                rgb.push(chunk[1]);
                rgb.push(chunk[0]);
            }
        }

        Ok(Image {
            width: w as u32,
            height: h as u32,
            data: rgb,
        })
    }
}
