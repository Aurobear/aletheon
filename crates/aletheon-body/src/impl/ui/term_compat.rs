/// Terminal capability detection for broad Linux compatibility.
///
/// Detects color depth, Unicode support, and provides ASCII fallbacks.
use ratatui::style::Color;

/// Semantic color theme for the TUI.
#[derive(Debug, Clone)]
pub struct Theme {
    pub accent: Color,
    pub text: Color,
    pub text_muted: Color,
    pub background: Color,
    pub bg_panel: Color,
    pub bg_user: Color,
    pub border: Color,
    pub border_active: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub user_icon: Color,
    pub assistant_icon: Color,
    pub system_icon: Color,
    pub code_bg: Color,
}

impl Theme {
    pub fn dark(caps: &TermCaps) -> Self {
        if caps.true_color {
            Self {
                accent: Color::Rgb(217, 119, 87),       // warm copper
                text: Color::Rgb(230, 230, 230),
                text_muted: Color::Rgb(120, 120, 120),
                background: Color::Rgb(18, 18, 24),
                bg_panel: Color::Rgb(28, 28, 38),
                bg_user: Color::Rgb(35, 35, 55),         // subtle blue tint
                border: Color::Rgb(60, 60, 80),
                border_active: Color::Rgb(217, 119, 87),
                error: Color::Rgb(220, 80, 80),
                warning: Color::Rgb(220, 180, 60),
                success: Color::Rgb(80, 200, 120),
                user_icon: Color::Rgb(100, 180, 255),    // light blue
                assistant_icon: Color::Rgb(217, 119, 87), // accent
                system_icon: Color::Rgb(120, 120, 120),
                code_bg: Color::Rgb(22, 22, 32),
            }
        } else {
            Self {
                accent: Color::Yellow,
                text: Color::White,
                text_muted: Color::DarkGray,
                background: Color::Black,
                bg_panel: Color::Black,
                bg_user: Color::Black,
                border: Color::DarkGray,
                border_active: Color::Yellow,
                error: Color::Red,
                warning: Color::Yellow,
                success: Color::Green,
                user_icon: Color::Cyan,
                assistant_icon: Color::Yellow,
                system_icon: Color::DarkGray,
                code_bg: Color::Black,
            }
        }
    }
}

/// Terminal capabilities detected from environment.
#[derive(Debug, Clone)]
pub struct TermCaps {
    /// Supports 24-bit true color (Color::Rgb).
    pub true_color: bool,
    /// Supports Unicode characters (emoji, box drawing, braille).
    pub unicode: bool,
    /// Terminal width (columns).
    #[allow(dead_code)]
    pub width: u16,
    /// Terminal height (rows).
    #[allow(dead_code)]
    pub height: u16,
}

impl TermCaps {
    /// Detect terminal capabilities from environment variables.
    pub fn detect() -> Self {
        let true_color = detect_true_color();
        let unicode = detect_unicode();
        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));

        Self {
            true_color,
            unicode,
            width,
            height,
        }
    }

    /// Get the semantic color theme for this terminal.
    pub fn theme(&self) -> Theme {
        Theme::dark(self)
    }

    /// Get a color, falling back to nearest ANSI color if true color unsupported.
    pub fn color(&self, r: u8, g: u8, b: u8) -> Color {
        if self.true_color {
            Color::Rgb(r, g, b)
        } else {
            rgb_to_ansi(r, g, b)
        }
    }

    /// User role icon.
    pub fn icon_user(&self) -> &'static str {
        if self.unicode { ">> " } else { ">> " }
    }

    /// Assistant role icon.
    pub fn icon_assistant(&self) -> &'static str {
        if self.unicode { "## " } else { "## " }
    }

    /// System role icon.
    pub fn icon_system(&self) -> &'static str {
        if self.unicode { "[i] " } else { "[i] " }
    }

    /// Connected indicator.
    pub fn icon_connected(&self) -> &'static str {
        if self.unicode { "●" } else { "*" }
    }

    /// Disconnected indicator.
    pub fn icon_disconnected(&self) -> &'static str {
        if self.unicode { "○" } else { "o" }
    }

    /// Spinner animation frames.
    pub fn spinner_frames(&self) -> [&'static str; 4] {
        if self.unicode {
            ["⠋", "⠙", "⠹", "⠸"]
        } else {
            ["-", "\\", "|", "/"]
        }
    }

    /// Vertical separator for code blocks and blockquotes.
    pub fn vline(&self) -> &'static str {
        if self.unicode { "│" } else { "|" }
    }

    /// Bullet for list items.
    pub fn bullet(&self) -> &'static str {
        if self.unicode { "•" } else { "*" }
    }

    /// Horizontal line separator.
    pub fn hline(&self) -> &'static str {
        if self.unicode { "─" } else { "-" }
    }
}

/// Detect 24-bit true color support.
fn detect_true_color() -> bool {
    // COLORTERM=truecolor or COLORTERM=24bit
    if let Ok(ct) = std::env::var("COLORTERM") {
        let ct = ct.to_lowercase();
        if ct == "truecolor" || ct == "24bit" {
            return true;
        }
    }

    // Known terminals with true color support
    if let Ok(term_program) = std::env::var("TERM_PROGRAM") {
        let tp = term_program.to_lowercase();
        if tp.contains("iterm")
            || tp.contains("alacritty")
            || tp.contains("kitty")
            || tp.contains("wezterm")
            || tp.contains("hyper")
            || tp.contains("vscode")
            || tp.contains("ghostty")
        {
            return true;
        }
    }

    // TERM contains "256color" — likely supports true color
    if let Ok(term) = std::env::var("TERM") {
        if term.contains("256color") {
            // Most 256-color terminals also support true color,
            // but not all. Conservative: only if TERM_PROGRAM is also set.
            return std::env::var("TERM_PROGRAM").is_ok();
        }
    }

    // tmux/screen — true color requires specific config, assume no
    if let Ok(term) = std::env::var("TERM") {
        if term.starts_with("tmux") || term.starts_with("screen") {
            return false;
        }
    }

    // Default: assume no true color
    false
}

/// Detect Unicode support.
fn detect_unicode() -> bool {
    // LC_ALL / LANG containing UTF-8
    for var in &["LC_ALL", "LANG", "LC_CTYPE"] {
        if let Ok(val) = std::env::var(var) {
            if val.to_uppercase().contains("UTF-8") || val.to_uppercase().contains("UTF8") {
                return true;
            }
        }
    }

    // Known terminals that support Unicode
    if let Ok(term_program) = std::env::var("TERM_PROGRAM") {
        let tp = term_program.to_lowercase();
        if tp.contains("iterm")
            || tp.contains("alacritty")
            || tp.contains("kitty")
            || tp.contains("wezterm")
            || tp.contains("hyper")
            || tp.contains("vscode")
            || tp.contains("gnome")
            || tp.contains("konsole")
            || tp.contains("ghostty")
        {
            return true;
        }
    }

    // Linux console (TERM=linux) — no Unicode
    if let Ok(term) = std::env::var("TERM") {
        if term == "linux" {
            return false;
        }
    }

    // Default: assume Unicode on modern systems
    // (most Linux distros set UTF-8 locale by default)
    true
}

/// Convert RGB to nearest ANSI 16-color.
fn rgb_to_ansi(r: u8, g: u8, b: u8) -> Color {
    let max = r.max(g).max(b) as u16;
    let brightness = (r as u16 + g as u16 + b as u16) / 3;

    if max < 30 {
        Color::Black
    } else if r >= g && r >= b {
        if max > 180 { Color::LightRed } else { Color::Red }
    } else if g >= r && g >= b {
        if max > 180 { Color::LightGreen } else { Color::Green }
    } else if b >= r && b >= g {
        if max > 180 { Color::LightBlue } else { Color::Blue }
    } else if brightness > 180 {
        Color::White
    } else if brightness > 100 {
        Color::DarkGray
    } else {
        Color::Black
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_to_ansi_red() {
        assert_eq!(rgb_to_ansi(255, 0, 0), Color::LightRed);
        assert_eq!(rgb_to_ansi(128, 0, 0), Color::Red);
    }

    #[test]
    fn test_rgb_to_ansi_green() {
        assert_eq!(rgb_to_ansi(0, 255, 0), Color::LightGreen);
    }

    #[test]
    fn test_rgb_to_ansi_dark() {
        assert_eq!(rgb_to_ansi(10, 10, 10), Color::Black);
    }

    #[test]
    fn test_term_caps_ascii_fallback() {
        let caps = TermCaps {
            true_color: false,
            unicode: false,
            width: 80,
            height: 24,
        };
        assert_eq!(caps.icon_user(), ">> ");
        assert_eq!(caps.icon_assistant(), "## ");
        assert_eq!(caps.icon_connected(), "*");
        assert_eq!(caps.vline(), "|");
        assert_eq!(caps.bullet(), "*");
    }
}
