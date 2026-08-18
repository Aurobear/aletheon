use aletheon::daemon::turn_engine::map_turn_execution;
use application::turn::coordinator::TurnExecution;

fn execution(stop: ::contracts::TurnStop, items: Vec<::contracts::ItemPayload>) -> TurnExecution {
    let usage =
        ::contracts::InferenceUsage::aggregate(items.iter().filter_map(|item| match item {
            ::contracts::ItemPayload::InferenceReceipt { receipt } => Some(&receipt.usage),
            _ => None,
        }));
    TurnExecution {
        result: ::contracts::TurnResult {
            output: format!("{stop:?}"),
            failure: (stop == ::contracts::TurnStop::Failed).then(|| ::contracts::TurnFailure {
                kind: ::contracts::TurnFailureKind::Runtime,
                message: "runtime failed".into(),
                retryable: false,
            }),
            stop,
            usage,
            metrics: ::contracts::TurnMetrics {
                tool_calls_made: 3,
                elapsed_ms: 42,
                ..Default::default()
            },
        },
        items,
        projection: None,
        context_projection: None,
        evaluation_artifacts: Default::default(),
    }
}

fn inference_receipt(
    inference_id: &str,
    usage: ::contracts::InferenceUsage,
) -> ::contracts::ItemPayload {
    ::contracts::ItemPayload::InferenceReceipt {
        receipt: ::contracts::types::inference_receipt::InferenceTerminalReceipt {
            schema_version:
                ::contracts::types::inference_receipt::INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1,
            inference_id: inference_id.into(),
            operation_id: "operation-1".into(),
            provider_id: "provider-1".into(),
            model_id: "model-1".into(),
            system_prefix_digest: "system-digest".into(),
            tool_schema_digest: "tool-digest".into(),
            status: ::contracts::types::inference_receipt::InferenceTerminalStatus::Succeeded,
            usage,
            context_capacity_tokens: None,
            active_context_occupancy_tokens: None,
            failure_kind: None,
            prefix_shape_digest: None,
            local_cache_miss_reason: None,
        },
    }
}

#[test]
fn turn_result_stop_mapping_is_exhaustive_and_lossless() {
    for stop in [
        ::contracts::TurnStop::Completed,
        ::contracts::TurnStop::Blocked,
        ::contracts::TurnStop::Cancelled,
        ::contracts::TurnStop::Failed,
    ] {
        let turn_id = ::contracts::TurnId::new();
        let mapped = map_turn_execution(turn_id, execution(stop.clone(), Vec::new()));
        assert_eq!(mapped.stop, stop);
        assert_eq!(mapped.output, format!("{stop:?}"));
        assert_eq!(mapped.tool_calls, 3);
        assert_eq!(mapped.elapsed_ms, 42);
        assert_eq!(mapped.turn_id, turn_id);
        assert!(mapped.coordinator_execution.is_some());
        assert_eq!(
            mapped.failure.is_some(),
            stop == ::contracts::TurnStop::Failed
        );
    }
}

#[test]
fn usage_is_summed_from_provider_receipts_without_fabricating_unknown_values() {
    let turn_id = ::contracts::TurnId::new();
    let mapped = map_turn_execution(
        turn_id,
        execution(
            ::contracts::TurnStop::Completed,
            vec![
                inference_receipt(
                    "inference-1",
                    ::contracts::InferenceUsage::reported(100, 20, Some(75), Some(25), Some(5)),
                ),
                inference_receipt(
                    "inference-2",
                    ::contracts::InferenceUsage::reported(60, 10, Some(50), Some(10), Some(2)),
                ),
            ],
        ),
    );

    assert_eq!(mapped.usage.total_input_tokens, Some(160));
    assert_eq!(mapped.usage.output_tokens, Some(30));
    assert_eq!(mapped.usage.uncached_input_tokens, Some(125));
    assert_eq!(mapped.usage.cache_read_tokens, Some(35));
    assert_eq!(mapped.usage.cache_write_tokens, Some(7));
    assert_eq!(
        mapped.usage.cache_telemetry,
        ::contracts::CacheTelemetry::Reported
    );

    let unknown = map_turn_execution(
        ::contracts::TurnId::new(),
        execution(
            ::contracts::TurnStop::Completed,
            vec![inference_receipt(
                "inference-unknown",
                ::contracts::InferenceUsage::default(),
            )],
        ),
    );
    assert_eq!(unknown.usage.total_input_tokens, None);
    assert_eq!(unknown.usage.output_tokens, None);
    assert_eq!(unknown.usage.cache_read_tokens, None);
    assert_eq!(unknown.usage.cache_write_tokens, None);
    assert_eq!(
        unknown.usage.cache_telemetry,
        ::contracts::CacheTelemetry::Unknown
    );
}
