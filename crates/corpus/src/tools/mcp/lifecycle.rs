//! Pure MCP connection and authorization lifecycle reducers.

use super::supervisor::McpServerHealthState;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpConnectionEvent {
    Register,
    ConnectionEstablished,
    PingHealthy,
    Reconnect,
    Degrade,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpConnectionEffect {
    Connect,
    PublishHealthy,
    ScheduleReconnect,
    PublishDegraded,
    StopTasks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConnectionTransition {
    pub previous: Option<McpServerHealthState>,
    pub next_state: McpServerHealthState,
    pub effects: Vec<McpConnectionEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidMcpConnectionTransition {
    pub previous: Option<McpServerHealthState>,
    pub event: McpConnectionEvent,
}

impl fmt::Display for InvalidMcpConnectionTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid MCP connection transition {:?} + {:?}",
            self.previous, self.event
        )
    }
}
impl std::error::Error for InvalidMcpConnectionTransition {}

pub fn reduce_mcp_connection(
    previous: Option<McpServerHealthState>,
    event: McpConnectionEvent,
) -> Result<McpConnectionTransition, InvalidMcpConnectionTransition> {
    use McpConnectionEffect::*;
    use McpConnectionEvent::*;
    use McpServerHealthState::*;
    let (next_state, effects) = match (previous, event) {
        (None, Register) => (Connecting, vec![Connect]),
        (None, PingHealthy | ConnectionEstablished) => (Connected, vec![PublishHealthy]),
        (None, Reconnect) => (Reconnecting, vec![ScheduleReconnect]),
        (None, Degrade) => (Degraded, vec![PublishDegraded]),
        (Some(Connecting | Reconnecting | Degraded), ConnectionEstablished) => {
            (Connected, vec![PublishHealthy])
        }
        (Some(Connected | Connecting | Reconnecting | Degraded), PingHealthy) => {
            (Connected, vec![PublishHealthy])
        }
        (Some(Connecting | Connected | Reconnecting | Degraded), Reconnect) => {
            (Reconnecting, vec![ScheduleReconnect])
        }
        (Some(Connecting | Connected | Reconnecting | Degraded), Degrade) => {
            (Degraded, vec![PublishDegraded])
        }
        (Some(Connecting | Connected | Reconnecting | Degraded), Shutdown) => {
            (Stopped, vec![StopTasks])
        }
        _ => return Err(InvalidMcpConnectionTransition { previous, event }),
    };
    Ok(McpConnectionTransition {
        previous,
        next_state,
        effects,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpOAuthState {
    Unauthenticated,
    Authorizing,
    Exchanging,
    Authorized,
    Refreshing,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpOAuthEvent {
    BeginAuthorization,
    ExchangeCode,
    TokenStored,
    BeginRefresh,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpOAuthEffect {
    PersistPendingState,
    ExchangeToken,
    PersistToken,
    RefreshToken,
    DenyCredentialRelease,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOAuthTransition {
    pub previous: McpOAuthState,
    pub next_state: McpOAuthState,
    pub effects: Vec<McpOAuthEffect>,
}

pub fn reduce_mcp_oauth(
    previous: McpOAuthState,
    event: McpOAuthEvent,
) -> Result<McpOAuthTransition, &'static str> {
    use McpOAuthEffect::*;
    use McpOAuthEvent::*;
    use McpOAuthState::*;
    let (next_state, effects) = match (previous, event) {
        (Unauthenticated | Authorizing | Authorized | Failed, BeginAuthorization) => {
            (Authorizing, vec![PersistPendingState])
        }
        (Authorizing, ExchangeCode) => (Exchanging, vec![ExchangeToken]),
        (Exchanging | Refreshing, TokenStored) => (Authorized, vec![PersistToken]),
        (Authorized | Failed, BeginRefresh) => (Refreshing, vec![RefreshToken]),
        (Authorizing | Exchanging | Refreshing, Fail) => (Failed, vec![DenyCredentialRelease]),
        _ => return Err("invalid MCP OAuth lifecycle transition"),
    };
    Ok(McpOAuthTransition {
        previous,
        next_state,
        effects,
    })
}

#[derive(Debug)]
pub struct McpOAuthLifecycle {
    state: McpOAuthState,
}

impl McpOAuthLifecycle {
    pub fn new(has_token: bool) -> Self {
        Self {
            state: if has_token {
                McpOAuthState::Authorized
            } else {
                McpOAuthState::Unauthenticated
            },
        }
    }
    pub fn state(&self) -> McpOAuthState {
        self.state
    }
    pub fn apply(&mut self, event: McpOAuthEvent) -> Result<McpOAuthTransition, &'static str> {
        let transition = reduce_mcp_oauth(self.state, event)?;
        self.state = transition.next_state;
        Ok(transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characterizes_connection_reconnect_degrade_and_shutdown() {
        use McpConnectionEvent::*;
        use McpServerHealthState::*;
        assert_eq!(
            reduce_mcp_connection(None, Register).unwrap().next_state,
            Connecting
        );
        assert_eq!(
            reduce_mcp_connection(Some(Connecting), ConnectionEstablished)
                .unwrap()
                .next_state,
            Connected
        );
        assert_eq!(
            reduce_mcp_connection(Some(Connected), Reconnect)
                .unwrap()
                .next_state,
            Reconnecting
        );
        assert_eq!(
            reduce_mcp_connection(Some(Reconnecting), Degrade)
                .unwrap()
                .next_state,
            Degraded
        );
        assert_eq!(
            reduce_mcp_connection(Some(Degraded), Shutdown)
                .unwrap()
                .next_state,
            Stopped
        );
        assert!(reduce_mcp_connection(Some(Stopped), Reconnect).is_err());
        assert!(reduce_mcp_connection(None, Shutdown).is_err());
    }

    #[test]
    fn characterizes_oauth_exchange_refresh_failure_and_replay() {
        use McpOAuthEvent::*;
        let mut lifecycle = McpOAuthLifecycle::new(false);
        lifecycle.apply(BeginAuthorization).unwrap();
        lifecycle.apply(ExchangeCode).unwrap();
        lifecycle.apply(TokenStored).unwrap();
        lifecycle.apply(BeginRefresh).unwrap();
        lifecycle.apply(TokenStored).unwrap();
        assert_eq!(lifecycle.state(), McpOAuthState::Authorized);
        assert!(lifecycle.apply(TokenStored).is_err());
        let mut failed = McpOAuthLifecycle::new(false);
        failed.apply(BeginAuthorization).unwrap();
        failed.apply(Fail).unwrap();
        assert_eq!(failed.state(), McpOAuthState::Failed);
    }
}
