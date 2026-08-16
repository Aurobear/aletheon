pub mod draw;
pub mod header;
pub mod input_line;
pub mod renderable;

use super::test_infra::FrameRecorder;
use super::{agent_inspector, history_search, pager, session_picker, state::AppState};
use super::{approval_dialog, checkpoint_picker, completion::CompletionPopup, diff_view};
use super::{status::StatusBar, streaming::StreamController, term_compat::TermCaps};
use ratatui::Terminal;

/// Immutable presentation snapshot borrowed from [`TuiModel`].
///
/// Rendering receives this narrow view instead of the application object, so
/// the renderer cannot issue Gateway commands, inspect transport state, read
/// the clock/environment, or mutate authoritative projection state.
pub(crate) struct TuiView<'a> {
    pub(crate) caps: &'a TermCaps,
    pub(crate) input_buf: &'a str,
    pub(crate) cursor: usize,
    pub(crate) has_cjk: bool,
    pub(crate) status: &'a StatusBar,
    pub(crate) pending_approval: &'a Option<approval_dialog::ApprovalDialog>,
    pub(crate) detail: &'a Option<diff_view::DiffView>,
    pub(crate) completion: &'a CompletionPopup,
    pub(crate) app_state: &'a AppState,
    pub(crate) stream_ctrl: &'a StreamController,
    pub(crate) pager: &'a Option<pager::PagerOverlay>,
    pub(crate) session_picker: &'a Option<session_picker::SessionPicker>,
    pub(crate) agent_inspector: &'a Option<agent_inspector::AgentInspector>,
    pub(crate) checkpoint_picker: &'a Option<checkpoint_picker::CheckpointPicker>,
    pub(crate) history_search: &'a Option<history_search::HistorySearchOverlay>,
    pub(crate) workspace_name: &'a str,
    pub(crate) selected_activity: Option<usize>,
    pub(crate) next_agent_runtime: Option<&'a str>,
}

/// Presentation-only renderer boundary. It receives already-reduced model
/// state and never opens sockets, issues commands, or derives authority.
pub(crate) struct TuiRenderer;

impl TuiRenderer {
    pub(crate) fn draw<B: ratatui::backend::Backend>(
        terminal: &mut Terminal<B>,
        view: &TuiView<'_>,
        frame_recorder: &mut Option<FrameRecorder>,
    ) -> anyhow::Result<()> {
        draw::draw_with_recorder(terminal, view, frame_recorder)
    }
}
