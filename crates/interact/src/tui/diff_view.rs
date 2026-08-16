use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use ::contracts::change_transaction::MutationCoverage;
use ::contracts::PatchDelta;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffFileEntry {
    pub path: String,
    pub change_type: String,
    pub hunks: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub binary: bool,
}

#[derive(Clone, Debug)]
pub struct DiffView {
    pub text: String,
    pub scroll: u16,
    pub files: Vec<DiffFileEntry>,
    pub selected_file: usize,
    pub coverage: Option<MutationCoverage>,
    pub conflicted: bool,
    /// Host-authored terminal/recovery decision. This is a projection only;
    /// the TUI never derives or persists settlement state.
    pub settlement: Option<::contracts::TransactionSettlementReceipt>,
    pub findings: Vec<::contracts::ReviewFinding>,
}

impl DiffView {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            scroll: 0,
            files: Vec::new(),
            selected_file: 0,
            coverage: None,
            conflicted: false,
            settlement: None,
            findings: Vec::new(),
        }
    }

    pub fn from_patch_delta(delta: &PatchDelta) -> Self {
        Self {
            text: delta.diff_preview.clone().unwrap_or_else(|| {
                delta
                    .files_changed
                    .iter()
                    .map(|file| format!("{} {}", file.change_type, file.path))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            scroll: 0,
            files: delta
                .files_changed
                .iter()
                .map(|file| DiffFileEntry {
                    path: file.path.clone(),
                    change_type: file.change_type.clone(),
                    hunks: file.hunks_applied,
                    bytes_before: file.bytes_before,
                    bytes_after: file.bytes_after,
                    binary: file.is_binary,
                })
                .collect(),
            selected_file: 0,
            coverage: delta.mutation_coverage,
            conflicted: !delta.failed.is_empty(),
            settlement: None,
            findings: Vec::new(),
        }
    }
    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }
    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }
    pub fn select_next(&mut self) {
        if !self.files.is_empty() {
            self.selected_file = (self.selected_file + 1) % self.files.len();
        }
    }
    pub fn select_previous(&mut self) {
        if !self.files.is_empty() {
            self.selected_file = self
                .selected_file
                .checked_sub(1)
                .unwrap_or(self.files.len() - 1);
        }
    }

    pub fn rollback_hint(&self) -> &'static str {
        match self.coverage {
            Some(MutationCoverage::Full) => "rollback: available",
            Some(MutationCoverage::BestEffort) => "rollback: best-effort (review required)",
            Some(MutationCoverage::NonRollbackable) => "rollback: unavailable (non-rollbackable)",
            None => "rollback: unknown (host review required)",
        }
    }

    pub fn project_settlement(&mut self, receipt: ::contracts::TransactionSettlementReceipt) {
        self.settlement = Some(receipt);
    }

    pub fn project_findings(&mut self, findings: Vec<::contracts::ReviewFinding>) {
        self.findings = findings;
    }
}

impl Widget for &DiffView {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let coverage = self
            .coverage
            .map(|value| format!("{value:?}").to_lowercase())
            .unwrap_or_else(|| "unknown".into());
        let terminal = if self.conflicted {
            "terminal: partial/conflicted; retry evidence retained"
        } else {
            "terminal: review pending"
        };
        let mut header = vec![
            Line::from(vec![
                Span::styled(" coverage ", Style::default().fg(Color::Yellow)),
                Span::styled(coverage, Style::default().fg(Color::White)),
                Span::styled(" · ", Style::default().fg(Color::DarkGray)),
                Span::styled(self.rollback_hint(), Style::default().fg(Color::Cyan)),
            ]),
            Line::from(Span::styled(
                terminal,
                Style::default().fg(if self.conflicted {
                    Color::Red
                } else {
                    Color::DarkGray
                }),
            )),
            Line::from(Span::styled(
                " actions: a accept · p repair · x rollback · f full diff · Esc close",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        if let Some(settlement) = &self.settlement {
            header.push(Line::from(vec![
                Span::styled(" host settlement ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("{:?}", settlement.decision).to_lowercase(),
                    Style::default().fg(Color::Green),
                ),
                Span::styled(" · ", Style::default().fg(Color::DarkGray)),
                Span::styled(settlement.reason.clone(), Style::default().fg(Color::White)),
            ]));
            if !settlement.finding_ids.is_empty() {
                header.push(Line::from(Span::styled(
                    format!(" findings: {}", settlement.finding_ids.join(", ")),
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
        for finding in &self.findings {
            let location = finding
                .location
                .as_ref()
                .map_or_else(String::new, |location| {
                    format!(
                        " @ {}{}",
                        location.path,
                        location
                            .line
                            .map(|line| format!(":{line}"))
                            .unwrap_or_default()
                    )
                });
            header.push(Line::from(Span::styled(
                format!(
                    " finding {} [{:?}/{:?}]{location}: {}",
                    finding.finding_id, finding.severity, finding.status, finding.summary
                ),
                Style::default().fg(
                    if finding.status == ::contracts::ReviewFindingStatus::Resolved {
                        Color::Green
                    } else {
                        Color::Red
                    },
                ),
            )));
        }
        if !self.files.is_empty() {
            header.push(Line::from(Span::styled(
                " files (j/k select; Esc close; f open full diff):",
                Style::default().fg(Color::DarkGray),
            )));
            for (index, file) in self.files.iter().enumerate() {
                let marker = if index == self.selected_file {
                    '>'
                } else {
                    ' '
                };
                let details = if file.binary {
                    "binary".to_string()
                } else {
                    format!(
                        "{} hunks, {}→{} bytes",
                        file.hunks, file.bytes_before, file.bytes_after
                    )
                };
                header.push(Line::from(Span::styled(
                    format!(" {marker} {} ({}) {details}", file.change_type, file.path),
                    Style::default().fg(if index == self.selected_file {
                        Color::White
                    } else {
                        Color::DarkGray
                    }),
                )));
            }
        }
        let mut old_line = 0u64;
        let mut new_line = 0u64;
        let mut lines = self
            .text
            .lines()
            .map(|text| {
                if let Some(header) = text.strip_prefix("@@") {
                    if let Some((old, new)) = parse_hunk_header(header) {
                        old_line = old;
                        new_line = new;
                    }
                }
                let (marker, old, new, color) = if text.starts_with('+') && !text.starts_with("+++")
                {
                    let current = new_line;
                    new_line += 1;
                    ('+', None, Some(current), Color::Green)
                } else if text.starts_with('-') && !text.starts_with("---") {
                    let current = old_line;
                    old_line += 1;
                    ('-', Some(current), None, Color::Red)
                } else {
                    let current_old = old_line;
                    let current_new = new_line;
                    if !text.starts_with("@@")
                        && !text.starts_with("---")
                        && !text.starts_with("+++")
                    {
                        old_line += 1;
                        new_line += 1;
                    }
                    (
                        ' ',
                        Some(current_old),
                        Some(current_new),
                        if text.starts_with("@@") {
                            Color::Cyan
                        } else {
                            Color::Gray
                        },
                    )
                };
                Line::from(vec![
                    Span::styled(
                        format!(
                            "{:>4} {:>4} {marker} ",
                            old.map_or(String::new(), |v| v.to_string()),
                            new.map_or(String::new(), |v| v.to_string())
                        ),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(text.to_string(), Style::default().fg(color)),
                ])
            })
            .collect::<Vec<_>>();
        header.append(&mut lines);
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Diff / Review ")
                    .borders(Borders::ALL),
            )
            .scroll((self.scroll, 0))
            .render(area, buffer);
    }
}

fn parse_hunk_header(header: &str) -> Option<(u64, u64)> {
    let body = header.trim().trim_end_matches('@').trim();
    let mut parts = body.split_whitespace();
    let old = parts
        .next()?
        .strip_prefix('-')?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    let new = parts
        .next()?
        .strip_prefix('+')?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    Some((old, new))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(coverage: Option<MutationCoverage>, failed: bool) -> PatchDelta {
        PatchDelta {
            transaction_id: None,
            mutation_coverage: coverage,
            applied: vec![],
            failed: if failed {
                vec![::contracts::PatchDeltaFailed {
                    operation: "write".into(),
                    path: "src/lib.rs".into(),
                    error: "external writer".into(),
                    hunks_applied_before_failure: Some(1),
                }]
            } else {
                vec![]
            },
            files_changed: vec![::contracts::PatchDeltaFileChange {
                path: "src/lib.rs".into(),
                change_type: "modified".into(),
                hunks_applied: 1,
                bytes_before: 10,
                bytes_after: 12,
                is_binary: false,
            }],
            diff_preview: Some("@@ -1 +1 @@\n-before\n+after".into()),
            diff_artifact: None,
            diff_preview_truncated: false,
        }
    }
    #[test]
    fn parses_hunk_coordinates() {
        assert_eq!(parse_hunk_header(" -12,2 +20,3 "), Some((12, 20)));
    }

    #[test]
    fn u_chk_005_best_effort_never_advertises_full_rollback() {
        let view = DiffView::from_patch_delta(&patch(Some(MutationCoverage::BestEffort), false));
        assert_eq!(
            view.rollback_hint(),
            "rollback: best-effort (review required)"
        );
        assert!(!view.rollback_hint().contains("available"));
    }

    #[test]
    fn u_chk_006_partial_failure_keeps_conflicted_terminal_evidence() {
        let view = DiffView::from_patch_delta(&patch(Some(MutationCoverage::Full), true));
        assert!(view.conflicted);
        assert_eq!(view.files[0].path, "src/lib.rs");
    }
}
