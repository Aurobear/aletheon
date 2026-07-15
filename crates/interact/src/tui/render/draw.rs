use ratatui::Terminal;

use super::super::test_infra::{buffer_to_text, now_ms, FrameRecorder, FrameSnapshot};
use super::super::App;
use super::renderable::{
    ChatRenderable, HeaderRenderable, InputRenderable, LayoutHelper, Renderable, StatusRenderable,
};

/// Draw with optional frame recording — captures the buffer inside the draw closure.
pub fn draw_with_recorder<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    frame_recorder: &mut Option<FrameRecorder>,
) -> anyhow::Result<()> {
    let chat_ref = &app.chat;
    let caps_ref = &app.caps;
    let input_buf = &app.input_buf;
    let cursor = app.cursor;
    let has_cjk = app.has_cjk;
    let status_ref = &app.status;
    let pending_approval_ref = &app.pending_approval;
    let completion_ref = &app.completion;
    let tool_count = app.chat.active_exec_count();
    let thinking_visible = app.stream_ctrl.is_thinking();
    let frame_counter = app.frame_counter;

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

        // Build composable layout: header | chat (flex) | input | status
        let header_rows: u16 = 1;
        let mut layout = LayoutHelper::new();
        layout.push_fixed(header_rows, HeaderRenderable { caps: caps_ref });
        layout.push_flex(ChatRenderable {
            chat: chat_ref,
            frame_counter,
            caps: caps_ref,
        });
        layout.push_fixed(
            2,
            InputRenderable {
                buf: input_buf,
                cursor,
                has_cjk,
                caps: caps_ref,
                completion: completion_ref,
            },
        );
        layout.push_fixed(1, StatusRenderable { status: status_ref });
        layout.render(size, f.buffer_mut());

        // Approval dialog rendered as modal overlay
        if let Some(ref dialog) = pending_approval_ref {
            dialog.render(f, size);
        }

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
