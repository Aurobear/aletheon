use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::Line,
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
    Terminal,
};

use super::super::App;
use super::super::test_infra::{FrameRecorder, FrameSnapshot, buffer_to_text, now_ms};
use super::header::render_header;
use super::input_line::render_input;

/// Draw with optional frame recording — captures the buffer inside the draw closure.
pub fn draw_with_recorder<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    frame_recorder: &mut Option<FrameRecorder>,
) -> anyhow::Result<()> {
    let chat_ref = &app.chat;
    let caps_ref = &app.caps;
    let model_name = &app.model_name;
    let input_buf = &app.input_buf;
    let cursor = app.cursor;
    let has_cjk = app.has_cjk;
    let first_render = app.first_render;
    let status_ref = &app.status;
    let pending_approval_ref = &app.pending_approval;
    let completion_ref = &app.completion;
    let tool_count = app.active_tools.len();
    let thinking_visible = app.stream_ctrl.is_thinking();
    let frame_counter = app.frame_counter;
    let active_tools_ref = &app.active_tools;

    let pager_ref = &app.pager;

    terminal.draw(|f| {
        let size = f.area();

        // If pager overlay is active, render it instead of normal UI
        if let Some(ref pager) = pager_ref {
            let mut pager_buf = ratatui::buffer::Buffer::empty(size);
            pager.render(size, &mut pager_buf);
            f.buffer_mut().merge(&pager_buf);
            return;
        }

        // Layout: header(2) | chat(min) | input(3) | status(1)
        let header_rows: u16 = if first_render { 3 } else { 1 };
        let input_rows: u16 = 3;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_rows),
                Constraint::Min(1),
                Constraint::Length(input_rows),
                Constraint::Length(1),
            ])
            .split(size);

        render_header(f, chunks[0], caps_ref, model_name, first_render);

        let chat_block = Block::default()
            .borders(Borders::NONE)
            .padding(Padding::horizontal(1));
        let chat_inner = chat_block.inner(chunks[1]);
        f.render_widget(chat_block, chunks[1]);

        // Use render_with_active_tools to include inline tool cards during streaming
        let chat_lines = chat_ref.render_with_active_tools(
            active_tools_ref,
            frame_counter,
            caps_ref,
        );
        let total_lines = chat_lines.len() as u16;
        let visible_height = chat_inner.height;
        let max_scroll = total_lines.saturating_sub(visible_height);
        let scroll = chat_ref.scroll_offset.min(max_scroll);
        let end = total_lines.saturating_sub(scroll);
        let start = end.saturating_sub(visible_height);
        let visible: Vec<Line> = chat_lines[start as usize..end as usize].to_vec();
        f.render_widget(
            Paragraph::new(visible).wrap(Wrap { trim: false }),
            chat_inner,
        );

        render_input(f, chunks[2], caps_ref, input_buf, cursor, has_cjk);

        f.render_widget(status_ref.render_widget(), chunks[3]);

        if let Some(ref dialog) = pending_approval_ref {
            dialog.render(f, size);
        }

        completion_ref.render(f, chunks[2]);

        // Record frame snapshot after all widgets are rendered
        if let Some(ref mut recorder) = frame_recorder {
            let snapshot = FrameSnapshot {
                ts: now_ms(),
                cols: size.width,
                rows: size.height,
                content: buffer_to_text(f.buffer_mut()),
                thinking_visible,
                tool_count,
            };
            recorder.write(&snapshot);
        }
    })?;

    app.first_render = false;
    app.frame_counter = app.frame_counter.wrapping_add(1);
    Ok(())
}
