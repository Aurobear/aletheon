use application::turn::ports::TurnIdentityPort;

/// The sole explicit representation bridge used after canonical-session
/// admission has verified that the authenticated thread names a durable
/// Runtime session.
pub struct CanonicalTurnIdentityAdapter;

impl TurnIdentityPort for CanonicalTurnIdentityAdapter {
    fn contract_session(
        &self,
        thread: &contracts::ThreadId,
    ) -> anyhow::Result<contracts::SessionId> {
        anyhow::ensure!(
            !thread.0.trim().is_empty(),
            "canonical session identity is empty"
        );
        Ok(contracts::SessionId(thread.0.clone()))
    }

    fn runtime_session(
        &self,
        session: &contracts::SessionId,
    ) -> anyhow::Result<runtime::SessionId> {
        anyhow::ensure!(
            !session.0.trim().is_empty(),
            "canonical session identity is empty"
        );
        Ok(runtime::SessionId(session.0.clone()))
    }

    fn runtime_turn(&self, turn: contracts::TurnId) -> anyhow::Result<runtime::TurnId> {
        Ok(runtime::TurnId(turn.0.to_string()))
    }
}
