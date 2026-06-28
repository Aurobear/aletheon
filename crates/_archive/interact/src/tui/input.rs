pub struct CommandHistory {
    entries: Vec<String>,
    cursor: usize,
    max_size: usize,
}

impl CommandHistory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            max_size: 50,
        }
    }

    pub fn push(&mut self, entry: String) {
        if entry.is_empty() { return; }
        if self.entries.last() == Some(&entry) { return; }
        self.entries.push(entry);
        if self.entries.len() > self.max_size {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len();
    }

    pub fn up(&mut self) -> Option<&str> {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.entries.get(self.cursor).map(|s| s.as_str())
        } else {
            None
        }
    }

    pub fn down(&mut self) -> Option<&str> {
        if self.cursor < self.entries.len() {
            self.cursor += 1;
            if self.cursor < self.entries.len() {
                self.entries.get(self.cursor).map(|s| s.as_str())
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn reset_cursor(&mut self) {
        self.cursor = self.entries.len();
    }
}
