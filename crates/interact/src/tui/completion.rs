//! Tab completion for slash commands.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem},
    Frame,
};

pub struct CompletionPopup {
    pub visible: bool,
    pub candidates: Vec<String>,
    pub selected: usize,
    pub input_prefix: String,
}

impl Default for CompletionPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletionPopup {
    pub fn new() -> Self {
        Self {
            visible: false,
            candidates: Vec::new(),
            selected: 0,
            input_prefix: String::new(),
        }
    }

    pub fn show(&mut self, prefix: &str, commands: &[String]) {
        self.candidates = commands
            .iter()
            .filter(|c| c.starts_with(prefix))
            .cloned()
            .collect();
        if self.candidates.is_empty() {
            self.visible = false;
            return;
        }
        self.visible = true;
        self.selected = 0;
        self.input_prefix = prefix.to_string();
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.candidates.clear();
    }

    pub fn next(&mut self) {
        if !self.candidates.is_empty() {
            self.selected = (self.selected + 1) % self.candidates.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.candidates.is_empty() {
            self.selected = if self.selected == 0 {
                self.candidates.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.candidates.get(self.selected).map(|s| s.as_str())
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible || self.candidates.is_empty() {
            return;
        }

        let items: Vec<ListItem> = self
            .candidates
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let style = if i == self.selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(Span::styled(format!("  {cmd} "), style)))
            })
            .collect();

        let height = (self.candidates.len() as u16 + 2).min(10);
        let popup = Rect {
            x: area.x + 2,
            y: area.y.saturating_sub(height),
            width: 30.min(area.width.saturating_sub(4)),
            height,
        };

        f.render_widget(Clear, popup);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let list = List::new(items).block(block);
        f.render_widget(list, popup);
    }
}
