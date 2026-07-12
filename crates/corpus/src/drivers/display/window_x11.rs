use anyhow::{Context, Result};
use x11rb::connection::Connection;

use super::window::{WindowInfo, WindowManager};

/// EWMH-based window manager using _NET_CLIENT_LIST, _NET_WM_NAME,
/// _NET_ACTIVE_WINDOW, and GetGeometry.
pub struct EwmhWindowManager;

impl Default for EwmhWindowManager {
    fn default() -> Self {
        Self::new()
    }
}

impl EwmhWindowManager {
    pub fn new() -> Self {
        Self
    }
}

/// Intern an atom by name, returning its Atom (u32) value.
fn intern(conn: &impl Connection, name: &[u8]) -> Result<u32> {
    use x11rb::protocol::xproto::intern_atom;
    let reply = intern_atom(conn, false, name)
        .context("intern_atom request failed")?
        .reply()
        .context("intern_atom reply failed")?;
    Ok(reply.atom)
}

impl WindowManager for EwmhWindowManager {
    fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        use x11rb::protocol::xproto::*;

        let (conn, screen_num) = x11rb::connect(None).context("Failed to connect to X11")?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

        let net_client_list = intern(&conn, b"_NET_CLIENT_LIST")?;
        let net_wm_name = intern(&conn, b"_NET_WM_NAME")?;
        let utf8_string = intern(&conn, b"UTF8_STRING")?;
        let net_active_window = intern(&conn, b"_NET_ACTIVE_WINDOW")?;
        let wm_class = intern(&conn, b"WM_CLASS")?;

        // Get the active window
        let active_reply = get_property(
            &conn,
            false,
            root,
            net_active_window as Atom,
            AtomEnum::WINDOW,
            0,
            1,
        )?
        .reply()?;
        let active_window: u32 = active_reply
            .value32()
            .and_then(|mut v| v.next())
            .unwrap_or(0);

        // Get client list
        let prop = get_property(
            &conn,
            false,
            root,
            net_client_list as Atom,
            AtomEnum::WINDOW,
            0,
            1024,
        )?
        .reply()?;

        let window_ids: Vec<u32> = match prop.value32() {
            Some(vals) => vals.collect(),
            None => return Ok(vec![]),
        };

        let mut windows = Vec::new();

        for &win_id in &window_ids {
            // Get title via _NET_WM_NAME (UTF8_STRING)
            let title = match get_property(
                &conn,
                false,
                win_id,
                net_wm_name as Atom,
                utf8_string as Atom,
                0,
                1024,
            )?
            .reply()
            {
                Ok(r) => r
                    .value8()
                    .map(|b| String::from_utf8_lossy(&b.collect::<Vec<_>>()).to_string())
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };

            // Get app name via WM_CLASS (two null-terminated strings; we want the second)
            let app_name = match get_property(
                &conn,
                false,
                win_id,
                wm_class as Atom,
                AtomEnum::STRING,
                0,
                256,
            )?
            .reply()
            {
                Ok(r) => r
                    .value8()
                    .map(|b| {
                        let bytes: Vec<u8> = b.collect();
                        let parts: Vec<&str> = bytes
                            .split(|&b| b == 0)
                            .filter(|s| !s.is_empty())
                            .filter_map(|s| std::str::from_utf8(s).ok())
                            .collect();
                        parts.last().unwrap_or(&"").to_string()
                    })
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };

            // Get geometry via GetGeometry
            let geom = get_geometry(&conn, win_id as Drawable)?.reply()?;

            windows.push(WindowInfo {
                id: win_id as u64,
                title,
                app_name,
                x: geom.x as i32,
                y: geom.y as i32,
                width: geom.width as i32,
                height: geom.height as i32,
                focused: win_id == active_window,
            });
        }

        Ok(windows)
    }

    fn focus_window(&self, id: u64) -> Result<()> {
        use x11rb::protocol::xproto::*;

        let (conn, screen_num) = x11rb::connect(None).context("Failed to connect to X11")?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

        let net_active_window = intern(&conn, b"_NET_ACTIVE_WINDOW")?;

        // Build _NET_ACTIVE_WINDOW client message (format 32)
        // data[0] = source indication (2 = pager)
        // data[1] = timestamp (0 = current)
        // data[2] = currently active window (0 = none)
        let data: [u32; 5] = [2, 0, 0, 0, 0];
        let event = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: id as u32,
            type_: net_active_window,
            data: data.into(),
        };

        send_event(
            &conn,
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )?
        .check()?;

        // Raise the window to the top of the stack
        configure_window(
            &conn,
            id as u32,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        )?
        .check()?;

        conn.flush()?;
        Ok(())
    }

    fn launch_app(&self, command: &str) -> Result<u32> {
        use std::process::Command;

        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            anyhow::bail!("Empty command");
        }

        let child = Command::new(parts[0])
            .args(&parts[1..])
            .spawn()
            .context(format!("Failed to launch '{}'", command))?;

        Ok(child.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ewhm_new() {
        let _wm = EwmhWindowManager::new();
    }

    #[test]
    fn test_launch_app_empty_command() {
        let wm = EwmhWindowManager::new();
        let result = wm.launch_app("");
        assert!(result.is_err());
    }
}
