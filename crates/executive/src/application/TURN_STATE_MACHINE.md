# Turn Pipeline state machine

`turn_lifecycle.rs` owns turn stage ordering. `TurnPipelineLifecycle::apply` is
the sole in-memory mutation entry; the pure reducer returns effects interpreted by
the existing application ports in `TurnPipeline::run`.

```text
Admission -> PreTurn -> CognitiveExecution -> ToolLoop
          -> PostTurn -> Projection -> Completed
                       \-> Failed | Cancelled
```

Admission covers identity/policy and workspace checkpoint creation. Pre-turn owns
lifecycle contributors, SelfField review, hooks, compaction-safe canonical context,
and user-item admission. Cognitive execution selects the model and composes governed
capabilities. Tool loop owns streaming, token accounting, tool calls, and ordered
terminal event normalization. Post-turn settles session and operation scope.
Projection writes canonical items, Agora/conscious outcomes, and lifecycle events.

Every active state accepts `Fail` and `Cancel`; terminal states accept no event, so
repeated completion and late events fail closed. Cancellation is driven by the
operation token. Timeouts use the same cancellation terminal. The workspace
checkpoint ID and canonical item dedupe keys provide recovery/idempotency; a restart
recovers through the checkpoint service rather than replaying partial in-memory state.

I/O remains behind context, session, capability, cognitive-session, projection,
checkpoint, event, and kernel ports. No reducer effect invokes a concrete harness,
tool, memory repository, or transport implementation.
