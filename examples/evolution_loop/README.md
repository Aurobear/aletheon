# Self-Evolution EventBus Loop Demo

Demonstrates the full closed loop for agent self-evolution through the EventBus.

## Architecture

```
ToolExecution --> [EventBus] --> CognitCore (reflect via LLM)
                                     |
                                     v
                              [ReflectionEvent]
                                     |
                              (batch threshold)
                                     |
                                     v
                            [RuleExtractedEvent]
                                     |
                              (failure threshold)
                                     |
                                     v
                        [EvolutionTriggeredEvent]
                                     |
                                     v
                        SelfField (validate mutations)
                                     |
                                     v
                         [MutationIntentEvent]
                                     |
                                     v
                    MetaRuntime (Morphogenesis Pipeline)
                                     |
                                     v
                          [EvolutionResultEvent]
```

## Prerequisites

```bash
export DEEPSEEK_API_KEY=your_key_here
```

If the API key is not set, the demo will print a warning and exit gracefully.

## Run

```bash
cargo run -p evolution_loop
```

## What It Does

1. Creates a `KernelEventBus` with async handler support
2. Configures an `LlmScheduler` with DeepSeek as the reflector provider
3. Subscribes a `ToolObservationHandler` (CognitCore) to `ToolObservationEvent`
4. Simulates 4 tool observations (1 success, 3 failures)
5. CognitCore reflects on each via LLM, extracts rules after batch threshold,
   and triggers evolution after consecutive failure threshold
6. Prints the full event flow as it happens
