use executive::application::daemon_turn_engine::map_pipeline_response;
use executive::application::turn_engine::TurnEngineStatus;

#[test]
fn maps_successful_pipeline_response_to_completed() {
    let turn_id = fabric::TurnId::new();
    let response = serde_json::json!({
        "result": {
            "response": "hi",
            "succeeded": true,
            "metrics": { "tool_calls_made": 3, "elapsed_ms": 42 }
        }
    });
    let result = map_pipeline_response(turn_id, &response);
    assert_eq!(result.status, TurnEngineStatus::Completed);
    assert_eq!(result.output, "hi");
    assert_eq!(result.tool_calls, 3);
    assert_eq!(result.elapsed_ms, 42);
    assert_eq!(result.turn_id, turn_id);
    assert!(result.coordinator_execution.is_none());
}

#[test]
fn maps_error_pipeline_response_to_blocked() {
    let turn_id = fabric::TurnId::new();
    let response = serde_json::json!({
        "error": { "code": -32603, "message": "boom" }
    });
    let result = map_pipeline_response(turn_id, &response);
    assert_eq!(result.status, TurnEngineStatus::Blocked);
    assert_eq!(result.output, "");
}
