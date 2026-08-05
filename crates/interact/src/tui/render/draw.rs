use ratatui::Terminal;

use super::super::test_infra::{buffer_to_text, now_ms, FrameRecorder, FrameSnapshot};
use super::super::App;
use super::renderable::{
    HeaderRenderable, InputRenderable, LayoutHelper, Renderable, StatusRenderable,
    TaskConsoleRenderable,
};

/// Draw with optional frame recording — captures the buffer inside the draw closure.
pub fn draw_with_recorder<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    frame_recorder: &mut Option<FrameRecorder>,
) -> anyhow::Result<()> {
    // Derive discovery UI from the current input on every redraw.  This keeps
    // completion correct for key events, bracketed paste, IME replacement and
    // test/terminal backends that coalesce input differently.
    super::super::app::key_handler::refresh_command_completion(app);
    let caps_ref = &app.caps;
    let input_buf = &app.input_buf;
    let cursor = app.cursor;
    let has_cjk = app.has_cjk;
    let status_ref = &app.status;
    let pending_approval_ref = &app.pending_approval;
    let detail_ref = &app.detail;
    let completion_ref = &app.completion;
    let tool_count = app.chat.active_exec_count();
    let thinking_visible = app.stream_ctrl.is_thinking();

    let pager_ref = &app.pager;
    let session_picker_ref = &app.session_picker;
    let history_search_ref = &app.history_search;

    terminal.draw(|f| {
        let size = f.area();

        // If pager overlay is active, render it instead of normal UI
        if let Some(ref pager) = pager_ref {
            let mut pager_buf = ratatui::buffer::Buffer::empty(size);
            pager.render(size, &mut pager_buf);
            f.buffer_mut().merge(&pager_buf);
            return;
        }
        if let Some(ref picker) = session_picker_ref {
            let mut picker_buf = ratatui::buffer::Buffer::empty(size);
            picker.render(size, &mut picker_buf);
            f.buffer_mut().merge(&picker_buf);
            return;
        }

        // Build composable layout: header | chat (flex) | input | status
        let header_rows: u16 = 1;
        let mut layout = LayoutHelper::new();
        layout.push_fixed(
            header_rows,
            HeaderRenderable {
                caps: caps_ref,
                state: &app.app_state,
            },
        );
        layout.push_flex(TaskConsoleRenderable {
            caps: caps_ref,
            state: &app.app_state,
            workspace: &app.workspace,
            selected_activity: app.selected_activity,
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
        layout.push_fixed(
            1,
            StatusRenderable {
                status: status_ref,
                state: &app.app_state,
            },
        );
        layout.render(size, f.buffer_mut());
        if let Some(detail) = detail_ref {
            let (x, width) = if size.width >= 100 {
                (size.x + size.width * 55 / 100, size.width * 45 / 100)
            } else {
                (size.x, size.width)
            };
            let area =
                ratatui::layout::Rect::new(x, size.y + 1, width, size.height.saturating_sub(4));
            ratatui::widgets::Widget::render(detail, area, f.buffer_mut());
        }
        // Completion is an overlay, not part of the input widget's clipping
        // region. Render it last so later siblings cannot erase it.
        let input_area = ratatui::layout::Rect::new(
            size.x,
            size.y + size.height.saturating_sub(3),
            size.width,
            2,
        );
        completion_ref.render(f, input_area);

        if let Some(search) = history_search_ref {
            search.render(size, f.buffer_mut());
        }

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
