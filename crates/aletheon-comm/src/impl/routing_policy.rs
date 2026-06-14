use aletheon_abi::{EventType, Priority};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteAction {
    /// Deliver immediately to all subscribers.
    FastPath,
    /// Must be reviewed by SelfField before delivery.
    RequireSelfFieldReview,
}

pub struct RoutingPolicy;

impl RoutingPolicy {
    pub fn evaluate(event_type: &EventType, priority: &Priority) -> RouteAction {
        match priority {
            Priority::Critical => RouteAction::RequireSelfFieldReview,
            _ => match event_type {
                EventType::IdentityQuery
                | EventType::BoundaryCheck
                | EventType::ConflictDetected
                | EventType::RejectionIssued => RouteAction::RequireSelfFieldReview,
                _ => RouteAction::FastPath,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selffield_events_require_review() {
        assert_eq!(
            RoutingPolicy::evaluate(&EventType::IdentityQuery, &Priority::Normal),
            RouteAction::RequireSelfFieldReview
        );
        assert_eq!(
            RoutingPolicy::evaluate(&EventType::BoundaryCheck, &Priority::High),
            RouteAction::RequireSelfFieldReview
        );
        assert_eq!(
            RoutingPolicy::evaluate(&EventType::ConflictDetected, &Priority::Low),
            RouteAction::RequireSelfFieldReview
        );
        assert_eq!(
            RoutingPolicy::evaluate(&EventType::RejectionIssued, &Priority::Normal),
            RouteAction::RequireSelfFieldReview
        );
    }

    #[test]
    fn test_critical_always_requires_review() {
        assert_eq!(
            RoutingPolicy::evaluate(&EventType::UserIntent, &Priority::Critical),
            RouteAction::RequireSelfFieldReview
        );
        assert_eq!(
            RoutingPolicy::evaluate(&EventType::ToolError, &Priority::Critical),
            RouteAction::RequireSelfFieldReview
        );
    }

    #[test]
    fn test_normal_events_fast_path() {
        assert_eq!(
            RoutingPolicy::evaluate(&EventType::UserIntent, &Priority::Normal),
            RouteAction::FastPath
        );
        assert_eq!(
            RoutingPolicy::evaluate(&EventType::ToolError, &Priority::High),
            RouteAction::FastPath
        );
        assert_eq!(
            RoutingPolicy::evaluate(&EventType::ActionCompleted, &Priority::Low),
            RouteAction::FastPath
        );
    }
}
