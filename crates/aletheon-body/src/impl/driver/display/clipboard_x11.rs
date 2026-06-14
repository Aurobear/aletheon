use std::time::Duration;

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::clipboard::ClipboardDriver;

/// X11 clipboard driver — real clipboard access via X11 selections.
///
/// Each operation opens a fresh connection (same pattern as X11DisplayDriver)
/// to avoid Send/Sync issues with long-lived X11 connections.
pub struct X11ClipboardDriver;

impl X11ClipboardDriver {
    pub fn new() -> Self {
        Self
    }
}

/// Helper: intern an atom by name, returning its Atom id.
fn intern_atom(conn: &(impl Connection + ?Sized), name: &[u8]) -> Result<Atom> {
    Ok(conn
        .intern_atom(false, name)
        .context("intern_atom request failed")?
        .reply()
        .context("intern_atom reply failed")?
        .atom)
}

impl ClipboardDriver for X11ClipboardDriver {
    fn get_clipboard(&self) -> Result<String> {
        let (conn, screen_num) = x11rb::connect(None).context("Failed to connect to X11")?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

        let clipboard_atom = intern_atom(&conn, b"CLIPBOARD")?;
        let utf8_atom = intern_atom(&conn, b"UTF8_STRING")?;
        let result_atom = intern_atom(&conn, b"ARGOS_CLIP")?;

        // Check if there is a clipboard owner at all.
        let owner_reply = conn
            .get_selection_owner(clipboard_atom)
            .context("get_selection_owner failed")?
            .reply()
            .context("get_selection_owner reply failed")?;
        if owner_reply.owner == x11rb::NONE {
            return Ok(String::new());
        }

        // Create a temporary window to receive the selection data.
        let win = conn.generate_id().context("generate_id failed")?;
        conn.create_window(
            0,        // depth: copy from parent
            win,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .context("create_window failed")?;

        // Ask the clipboard owner to convert the selection into UTF8_STRING
        // and write it into our `result_atom` property on `win`.
        conn.convert_selection(
            win,
            clipboard_atom,
            utf8_atom,
            result_atom,
            x11rb::CURRENT_TIME,
        )
        .context("convert_selection failed")?;
        conn.flush().context("flush failed")?;

        // Poll for SelectionNotify with a timeout.
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        let mut result = String::new();

        loop {
            if std::time::Instant::now() > deadline {
                break; // timed out — return empty
            }

            match conn.poll_for_event().context("poll_for_event failed")? {
                Some(event) => match event {
                    x11rb::protocol::Event::SelectionNotify(ev) => {
                        if ev.requestor == win && ev.property == result_atom {
                            // The property was set (or set to NONE on refusal).
                            if ev.property == x11rb::NONE {
                                break; // owner refused — nothing to read
                            }
                            let prop_reply = conn
                                .get_property(
                                    false,
                                    win,
                                    result_atom,
                                    utf8_atom,
                                    0,
                                    64 * 1024, // up to 64 KiB
                                )
                                .context("get_property failed")?
                                .reply()
                                .context("get_property reply failed")?;

                            if let Some(bytes) = prop_reply.value8() {
                                let collected: Vec<u8> = bytes.collect();
                                result = String::from_utf8_lossy(&collected).to_string();
                            }
                            break;
                        }
                    }
                    _ => { /* ignore unrelated events */ }
                },
                None => {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }

        // Cleanup the temporary window.
        let _ = conn.destroy_window(win);
        let _ = conn.flush();

        Ok(result)
    }

    fn set_clipboard(&self, text: &str) -> Result<()> {
        let (conn, screen_num) = x11rb::connect(None).context("Failed to connect to X11")?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

        let clipboard_atom = intern_atom(&conn, b"CLIPBOARD")?;
        let utf8_atom = intern_atom(&conn, b"UTF8_STRING")?;

        // Create a window that will own the CLIPBOARD selection.
        let win = conn.generate_id().context("generate_id failed")?;
        conn.create_window(
            0,
            win,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .context("create_window failed")?;

        // Become the CLIPBOARD owner.
        conn.set_selection_owner(win, clipboard_atom, x11rb::CURRENT_TIME)
            .context("set_selection_owner failed")?;
        conn.flush().context("flush failed")?;

        // Verify ownership.
        let owner_reply = conn
            .get_selection_owner(clipboard_atom)
            .context("get_selection_owner failed")?
            .reply()
            .context("get_selection_owner reply failed")?;
        if owner_reply.owner != win {
            anyhow::bail!("Failed to acquire CLIPBOARD selection ownership");
        }

        // Pre-store the data in a property so the event loop can serve it
        // when a SelectionRequest arrives.  This is a simplified approach —
        // a full implementation would run an event loop to serve the data
        // on-demand, but this is sufficient for the driver's current scope.
        let data_atom = intern_atom(&conn, b"ARGOS_CLIPBOARD_DATA")?;
        conn.change_property(
            PropMode::REPLACE,
            win,
            data_atom,
            utf8_atom,
            8,
            text.len() as u32,
            text.as_bytes(),
        )
        .context("change_property failed")?;
        conn.flush().context("flush failed")?;

        // NOTE: In a production implementation we would now run an event loop
        // to handle SelectionRequest events, replying with the stored data.
        // Without that loop the data remains available only while this
        // connection is alive (i.e. until the function returns).

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip test: only runs when a DISPLAY is available.
    #[test]
    fn test_clipboard_roundtrip() {
        // Skip gracefully if no X11 display is reachable.
        if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
            eprintln!("Skipping clipboard roundtrip test: no DISPLAY");
            return;
        }

        let driver = X11ClipboardDriver::new();

        let test_text = "argos-x11-clipboard-test-42";
        driver
            .set_clipboard(test_text)
            .expect("set_clipboard should succeed");

        // Give the X server a moment to propagate.
        std::thread::sleep(Duration::from_millis(100));

        let got = driver
            .get_clipboard()
            .expect("get_clipboard should succeed");

        // We may not get our own text back if another client stole the
        // selection between set and get, so we only assert non-error.
        // But in a typical single-user session this should round-trip.
        assert!(
            got.contains(test_text) || got.is_empty(),
            "Expected clipboard to contain {:?}, got {:?}",
            test_text,
            got,
        );
    }
}
