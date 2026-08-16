use ratatui::Terminal;

use super::super::test_infra::{buffer_to_text, now_ms, FrameRecorder, FrameSnapshot};
use super::renderable::{
    HeaderRenderable, InputRenderable, LayoutHelper, Renderable, StatusRenderable,
    TaskConsoleRenderable,
};
use super::TuiView;

/// Draw with optional frame recording — captures the buffer inside the draw closure.
pub fn draw_with_recorder<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    view: &TuiView<'_>,
    frame_recorder: &mut Option<FrameRecorder>,
) -> anyhow::Result<()> {
    let caps_ref = view.caps;
    let input_buf = view.input_buf;
    let cursor = view.cursor;
    let has_cjk = view.has_cjk;
    let status_ref = view.status;
    let pending_approval_ref = view.pending_approval;
    let detail_ref = view.detail;
    let completion_ref = view.completion;
    let tool_count = view
        .app_state
        .activities
        .iter()
        .filter(|activity| activity.kind == ::contracts::protocol::client::ActivityKind::Tool)
        .count();
    let thinking_visible = view.stream_ctrl.is_thinking();

    let pager_ref = view.pager;
    let session_picker_ref = view.session_picker;
    let agent_inspector_ref = view.agent_inspector;
    let checkpoint_picker_ref = view.checkpoint_picker;
    let history_search_ref = view.history_search;

    terminal.draw(|f| {
        let size = f.area();

        // If an overlay is active, render it instead of normal UI. Keep one
        // closure path so frame recording also captures modal frames.
        let mut overlay_rendered = false;
        if let Some(ref pager) = pager_ref {
            let mut pager_buf = ratatui::buffer::Buffer::empty(size);
            pager.render(size, &mut pager_buf);
            f.buffer_mut().merge(&pager_buf);
            overlay_rendered = true;
        }
        if !overlay_rendered {
            if let Some(ref picker) = session_picker_ref {
                let mut picker_buf = ratatui::buffer::Buffer::empty(size);
                picker.render(size, &mut picker_buf);
                f.buffer_mut().merge(&picker_buf);
                overlay_rendered = true;
            }
        }
        if !overlay_rendered {
            if let Some(ref inspector) = agent_inspector_ref {
                let mut inspector_buf = ratatui::buffer::Buffer::empty(size);
                inspector.render(size, &mut inspector_buf);
                f.buffer_mut().merge(&inspector_buf);
                overlay_rendered = true;
            }
        }
        if !overlay_rendered {
            if let Some(ref picker) = checkpoint_picker_ref {
                let mut picker_buf = ratatui::buffer::Buffer::empty(size);
                picker.render(size, &mut picker_buf);
                f.buffer_mut().merge(&picker_buf);
                overlay_rendered = true;
            }
        }

        if overlay_rendered {
            // The shared recorder below must run for overlays too.
        } else {
            // Build composable layout: header | chat (flex) | input | status
            let header_rows: u16 = 1;
            let mut layout = LayoutHelper::new();
            layout.push_fixed(
                header_rows,
                HeaderRenderable {
                    caps: caps_ref,
                    state: view.app_state,
                },
            );
            layout.push_flex(TaskConsoleRenderable {
                caps: caps_ref,
                state: view.app_state,
                workspace_name: view.workspace_name,
                selected_activity: view.selected_activity,
                next_agent_runtime: view.next_agent_runtime,
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
                    state: view.app_state,
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

    Ok(())
}
