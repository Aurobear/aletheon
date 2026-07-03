use super::ReActLoop;
use base::self_field::SelfState;
use cognit::core::awareness_signal::{self, AwarenessSignal, StepType};

impl ReActLoop {
    /// Emit an awareness signal into the collection buffer.
    pub(crate) fn emit_signal(&mut self, signal: AwarenessSignal) {
        self.signals.push(signal);
    }

    /// Drain collected awareness signals, leaving the buffer empty.
    pub fn take_signals(&mut self) -> Vec<AwarenessSignal> {
        std::mem::take(&mut self.signals)
    }

    /// Drain accumulated awareness signals as UI events.
    ///
    /// Returns `(AwarenessLevel, context)` pairs suitable for TUI display.
    /// Signals with no detected state or unrecognized states are filtered out.
    pub fn drain_awareness_events(&mut self) -> Vec<(base::ui_event::AwarenessLevel, String)> {
        let signals: Vec<_> = self.signals.drain(..).collect();
        awareness_signal::signals_to_ui_events(&signals)
    }

    /// Emit a LoopStart signal with impasse detection.
    pub(crate) fn emit_loop_start(&mut self, action: &str) {
        use cognit::core::awareness_signal::detect_impasse;
        let detected = detect_impasse(
            self.consecutive_errors,
            self.iteration,
            self.config.max_iterations,
        );
        self.emit_signal(AwarenessSignal {
            step: StepType::LoopStart,
            action: action.to_string(),
            detected_state: detected,
            timestamp: chrono::Utc::now(),
        });
    }

    /// Emit a ThinkingComplete signal with uncertainty detection from response text.
    pub(crate) fn emit_thinking_complete(&mut self, action: &str, response_text: &str) {
        use cognit::core::awareness_signal::detect_uncertainty;
        let detected = detect_uncertainty(response_text);
        self.emit_signal(AwarenessSignal {
            step: StepType::ThinkingComplete,
            action: action.to_string(),
            detected_state: detected,
            timestamp: chrono::Utc::now(),
        });
    }

    /// Emit a ToolCallEnd signal with impasse detection from consecutive errors.
    pub(crate) fn emit_tool_call_end(&mut self, tool_name: &str) {
        use cognit::core::awareness_signal::{detect_goal_shift, detect_impasse};

        // Track tool name for goal-shift detection
        self.recent_tools.push(tool_name.to_string());

        let mut detected = None;

        // Check impasse from errors
        if let Some(state) = detect_impasse(
            self.consecutive_errors,
            self.iteration,
            self.config.max_iterations,
        ) {
            detected = Some(state);
        }

        // Check goal shift from tool sequence
        if detected.is_none() {
            detected = detect_goal_shift(&self.recent_tools);
        }

        self.emit_signal(AwarenessSignal {
            step: StepType::ToolCallEnd,
            action: format!("tool:{}", tool_name),
            detected_state: detected,
            timestamp: chrono::Utc::now(),
        });
    }

    /// Emit a FinalResponse signal with Focused state.
    pub(crate) fn emit_final_response(&mut self, action: &str) {
        self.emit_signal(AwarenessSignal {
            step: StepType::FinalResponse,
            action: action.to_string(),
            detected_state: Some(SelfState::Focused),
            timestamp: chrono::Utc::now(),
        });
    }
}
