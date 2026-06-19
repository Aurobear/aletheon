use crossterm::event::{Event, KeyEvent};

/// Events processed by the TUI main loop.
pub enum TuiEvent {
    /// Terminal key press.
    Key(KeyEvent),
    /// Bracketed paste payload.
    Paste(String),
    /// Terminal was resized.
    Resize,
    /// Periodic tick for streaming redraws.
    Tick,
}

/// Actions produced by handling a TuiEvent.
pub enum Action {
    /// User submitted a message.
    Submit(String),
    /// User entered a /command.
    Command(String),
    /// User wants to quit.
    Quit,
    /// Scroll chat up by N lines.
    ScrollUp(u16),
    /// Scroll chat down by N lines.
    ScrollDown(u16),
    /// No action.
    None,
}

impl TuiEvent {
    /// Convert a crossterm Event into a TuiEvent.
    pub fn from_crossterm(event: Event) -> Option<Self> {
        match event {
            Event::Key(key) => Some(TuiEvent::Key(key)),
            Event::Paste(s) => Some(TuiEvent::Paste(s)),
            Event::Resize(_, _) => Some(TuiEvent::Resize),
            crossterm::event::Event::Mouse(mouse) => match mouse.kind {
                crossterm::event::MouseEventKind::ScrollUp => {
                    Some(TuiEvent::Key(crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::PageUp,
                        crossterm::event::KeyModifiers::empty(),
                    )))
                }
                crossterm::event::MouseEventKind::ScrollDown => {
                    Some(TuiEvent::Key(crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::PageDown,
                        crossterm::event::KeyModifiers::empty(),
                    )))
                }
                _ => None,
            },
            _ => None,
        }
    }
}
