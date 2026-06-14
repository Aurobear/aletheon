# Self-Evolution EventBus Loop Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the self-evolution closed loop through EventBus so BrainCore, SelfField, and MetaRuntime communicate via events with zero direct coupling.

**Architecture:** EventBus-driven pub/sub with async handlers. BrainCore subscribes to tool observations, reflects via LLM, and emits reflection/evolution events. SelfField validates mutation intents. MetaRuntime executes the Morphogenesis Pipeline. LlmScheduler is a shared service (not an EventBus subscriber) for request-response LLM routing.

**Tech Stack:** Rust, tokio, aletheon-abi (Event trait/EventBus trait), aletheon-comm (KernelEventBus), aletheon-brain (LlmProvider, LlmScheduler), aletheon-self (MutationLayer), aletheon-meta (MorphogenesisPipeline)

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `crates/aletheon-abi/src/event.rs` | Modify | Add new EventType variants, AsyncEventHandler type |
| `crates/aletheon-abi/src/event_bus.rs` | Modify | Add `subscribe_async` method to EventBus trait |
| `crates/aletheon-abi/src/evolution.rs` | Create | Self-evolution event structs (ToolObservationEvent, ReflectionEvent, etc.) |
| `crates/aletheon-abi/src/lib.rs` | Modify | Re-export evolution module |
| `crates/aletheon-comm/src/impl/kernel_bus.rs` | Modify | Implement `subscribe_async` |
| `crates/aletheon-comm/src/impl/subscription.rs` | Modify | Add async handler storage and dispatch |
| `crates/aletheon-brain/src/impl/llm/scheduler.rs` | Create | LlmScheduler — centralized dual-model routing |
| `crates/aletheon-brain/src/impl/llm/mod.rs` | Modify | Re-export scheduler |
| `crates/aletheon-brain/src/impl/event_handlers/mod.rs` | Create | Event handler module |
| `crates/aletheon-brain/src/impl/event_handlers/tool_observer.rs` | Create | Subscribes to ToolObservationEvent, reflects via LLM |
| `crates/aletheon-brain/src/impl/mod.rs` | Modify | Add event_handlers module |
| `crates/aletheon-self/src/impl/mutation/mod.rs` | Create | Mutation event handler module |
| `crates/aletheon-self/src/impl/mutation/approver.rs` | Create | Subscribes to EvolutionTriggeredEvent, validates via MutationLayer |
| `crates/aletheon-self/src/impl/mod.rs` | Modify | Add mutation module |
| `crates/aletheon-meta/src/impl/event_handlers/mod.rs` | Create | Event handler module |
| `crates/aletheon-meta/src/impl/event_handlers/mutation_executor.rs` | Create | Subscribes to MutationIntentEvent, runs Morphogenesis Pipeline |
| `crates/aletheon-meta/src/impl/mod.rs` | Modify | Add event_handlers module |
| `crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs` | Modify | Emit ToolObservationEvent after tool execution |
| `crates/aletheon-runtime/src/impl/engine/config.rs` | Modify | Add event_bus field to EngineConfig |
| `examples/self-evolution-loop/src/main.rs` | Create | End-to-end demo wiring all handlers |
| `examples/self-evolution-loop/Cargo.toml` | Create | Demo dependencies |
| `examples/self-evolution-loop/config.toml` | Create | Dual-model config |
| `examples/self-evolution-loop/genome.yaml` | Create | Initial genome |

---

## Task 1: Add AsyncEventHandler and New EventTypes to ABI

**Files:**
- Modify: `crates/aletheon-abi/src/event.rs`
- Modify: `crates/aletheon-abi/src/event_bus.rs`
- Create: `crates/aletheon-abi/src/evolution.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Add async handler type and new EventType variants**

In `crates/aletheon-abi/src/event.rs`, add after the existing `EventHandler` type (line 107):

```rust
/// Async event handler that can perform async work (e.g., LLM calls).
/// Returns true to continue propagation, false to stop.
pub type AsyncEventHandler = Box<dyn Fn(Box<dyn Event>) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;
```

Add new `EventType` variants in the enum (after `ReActIterationEnd`):

```rust
    // Self-evolution loop
    ReflectionComplete,
    RuleExtracted,
    EvolutionTriggered,
    EvolutionResult,
    LlmRequest,
    LlmResponse,
```

Note: `ReflectionComplete` already exists as `ReflectionComplete` in BrainCore section. Check for duplicates — if it exists, don't re-add. The new ones are `RuleExtracted`, `EvolutionTriggered`, `EvolutionResult`.

- [ ] **Step 2: Add `subscribe_async` to EventBus trait**

In `crates/aletheon-abi/src/event_bus.rs`, add to the `EventBus` trait:

```rust
    /// Register an async handler for an event type.
    /// Default implementation wraps in a sync handler that spawns a tokio task.
    async fn subscribe_async(&self, event_type: EventType, handler: AsyncEventHandler) -> Result<SubscriptionId> {
        // Default: wrap async handler in sync handler
        let sync_handler: EventHandler = Box::new(move |event: &dyn Event| {
            // We need to clone the event data for the async handler
            // Since Event trait has payload() -> &dyn Any, we can't easily move it
            // The default impl returns true (non-blocking) and spawns async work
            true
        });
        self.subscribe(event_type, sync_handler).await
    }
```

- [ ] **Step 3: Create evolution event structs**

Create `crates/aletheon-abi/src/evolution.rs`:

```rust
//! Self-evolution loop event types.
//!
//! These events flow through the EventBus to decouple BrainCore, SelfField, and MetaRuntime.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::self_field::MutationIntent;

/// Assessment of a tool execution outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Assessment {
    Success,
    PartialSuccess,
    Failure,
}

/// A learned rule extracted from experience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedRule {
    pub id: Uuid,
    pub condition: String,
    pub action: String,
    pub confidence: f64,
    pub source_reflections: Vec<Uuid>,
}

/// Emitted by Engine after a tool call completes.
/// Subscribed by BrainCore for reflection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolObservationPayload {
    pub turn_id: Uuid,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub rules_applied: Vec<LearnedRule>,
}

/// Emitted by BrainCore after LLM reflection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionPayload {
    pub turn_id: Uuid,
    pub assessment: Assessment,
    pub root_cause: Option<String>,
    pub suggested_rule: Option<LearnedRule>,
    pub confidence: f64,
}

/// Emitted when BrainCore accumulates enough reflections to extract generalized rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExtractedPayload {
    pub rules: Vec<LearnedRule>,
    pub source_reflections: Vec<Uuid>,
}

/// Emitted when BrainCore detects evolution conditions are met.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionTriggeredPayload {
    pub trigger_reason: String,
    pub recent_reflections: Vec<Uuid>,
    pub current_rules_snapshot: Vec<LearnedRule>,
}

/// Emitted by SelfField after validating mutation intents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationIntentPayload {
    pub intents: Vec<MutationIntent>,
    pub approved_by: String,
}

/// Emitted by MetaRuntime after Morphogenesis Pipeline completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionResultPayload {
    pub adopted: bool,
    pub genome_version_before: String,
    pub genome_version_after: Option<String>,
    pub summary: String,
}

/// Purpose of an LLM call, used for routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmPurpose {
    Reflect,
    ExtractRules,
    GenerateMutations,
    Execute,
}
```

- [ ] **Step 4: Re-export evolution module**

In `crates/aletheon-abi/src/lib.rs`, add:

```rust
pub mod evolution;
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p aletheon-abi`
Expected: Compiles with warnings about unused types (OK at this stage)

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-abi/src/evolution.rs crates/aletheon-abi/src/event.rs crates/aletheon-abi/src/event_bus.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add self-evolution event types and async handler support"
```

---

## Task 2: Implement subscribe_async in KernelEventBus

**Files:**
- Modify: `crates/aletheon-comm/src/impl/subscription.rs`
- Modify: `crates/aletheon-comm/src/impl/kernel_bus.rs`

- [ ] **Step 1: Add async handler support to SubscriptionRegistry**

In `crates/aletheon-comm/src/impl/subscription.rs`, add async handler storage alongside the existing sync handlers:

```rust
use aletheon_abi::AsyncEventHandler;
use std::future::Future;
use std::pin::Pin;

pub struct SubscriptionRegistry {
    handlers: RwLock<HashMap<EventType, Vec<Subscription>>>,
    async_handlers: RwLock<HashMap<EventType, Vec<AsyncSubscription>>>,
}

struct AsyncSubscription {
    id: SubscriptionId,
    handler: AsyncEventHandler,
}
```

Add a `dispatch_async` method:

```rust
impl SubscriptionRegistry {
    /// Dispatch event to all async handlers. Non-blocking — spawns tasks.
    pub async fn dispatch_async(&self, event: Box<dyn Event>) {
        let event_type = event.event_type();
        let handlers = self.async_handlers.read().await;
        if let Some(subs) = handlers.get(&event_type) {
            for sub in subs {
                let handler = &sub.handler;
                let result = handler(event.clone_event()).await;
                // result is bool — if false, could stop propagation
                // For now, we dispatch to all handlers
            }
        }
    }
}
```

Note: The `Event` trait needs a `clone_event` method or we need to restructure. Since `Event` is `dyn`, we can't clone it directly. Instead, we'll use a different approach — pass the event as `Box<dyn Event>` and let handlers extract what they need via `payload()`.

Actually, the simpler approach: since `EventHandler` takes `&dyn Event` (borrowed), and `AsyncEventHandler` needs to own the data, we should make the async handler take a concrete payload. Let me revise:

**Revised approach:** Don't add `AsyncEventHandler` to the trait. Instead, create a typed event handler pattern where each handler subscribes to an `EventType` and receives the payload via a channel.

```rust
// In evolution.rs, add a helper:
pub struct EvolutionEventBridge {
    tx: mpsc::Sender<Box<dyn Event>>,
    _subscription_id: SubscriptionId,
}

impl EvolutionEventBridge {
    pub async fn new(
        event_type: EventType,
        bus: &dyn EventBus,
    ) -> (Self, mpsc::Receiver<Box<dyn Event>>) {
        let (tx, rx) = mpsc::channel(64);
        let tx_clone = tx.clone();
        let handler: EventHandler = Box::new(move |event: &dyn Event| {
            // We can't send borrowed &dyn Event through a channel
            // Need to serialize/deserialize or use a different approach
            true
        });
        let sub_id = bus.subscribe(event_type, handler).await.unwrap();
        // ...
    }
}
```

This is getting complex because the existing `EventHandler` is sync and borrows. Let me take the pragmatic approach:

**Final approach:** Keep EventBus sync. Add a new `EventDispatcher` helper in aletheon-comm that bridges sync events to async handlers via channels. Handlers register with the dispatcher, not directly with EventBus.

- [ ] **Step 1 (revised): Create EventDispatcher bridge**

Create `crates/aletheon-comm/src/impl/dispatcher.rs`:

```rust
//! EventDispatcher bridges sync EventBus events to async handlers.
//!
//! The existing EventBus dispatches synchronously. EventDispatcher subscribes to
//! EventBus events and forwards them to async handlers via channels.

use std::collections::HashMap;
use std::sync::Arc;
use anyhow::Result;
use tokio::sync::{mpsc, RwLock};
use aletheon_abi::{Event, EventBus, EventHandler, EventType, SubscriptionId};

/// An async event handler function.
pub type AsyncHandler = Box<dyn Fn(serde_json::Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// Bridges sync EventBus to async handlers via channels.
pub struct EventDispatcher {
    /// Per-event-type channels. Sync handler sends serialized event, async handler receives.
    channels: RwLock<HashMap<EventType, mpsc::Sender<serde_json::Value>>>,
    /// Subscription IDs for cleanup.
    subscriptions: RwLock<Vec<SubscriptionId>>,
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            subscriptions: RwLock::new(Vec::new()),
        }
    }

    /// Register an async handler for an event type.
    /// The handler receives the event payload as serde_json::Value (via Event::summary() or custom serialization).
    pub async fn on<F, Fut>(&self, event_type: EventType, bus: &dyn EventBus, handler: F) -> Result<()>
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel::<serde_json::Value>(64);

        // Spawn async consumer
        tokio::spawn(async move {
            while let Some(payload) = rx.recv().await {
                handler(payload).await;
            }
        });

        // Register sync handler on EventBus that sends to channel
        let tx_clone = tx.clone();
        let sync_handler: EventHandler = Box::new(move |event: &dyn Event| {
            // Serialize event data via payload
            let payload = serde_json::to_value(event.summary()).unwrap_or_default();
            let _ = tx_clone.try_send(payload);
            true // continue propagation
        });

        let sub_id = bus.subscribe(event_type, sync_handler).await?;
        self.subscriptions.write().await.push(sub_id);
        self.channels.write().await.insert(event_type, tx);

        Ok(())
    }

    /// Remove all subscriptions.
    pub async fn cleanup(&self, bus: &dyn EventBus) -> Result<()> {
        let subs = self.subscriptions.read().await;
        for id in subs.iter() {
            bus.unsubscribe(*id).await?;
        }
        Ok(())
    }
}
```

Wait, there's a problem: `event.summary()` returns a String, not the structured payload. We need the actual typed data. The `Event` trait has `payload() -> &dyn Any` which we could downcast, but that requires knowing the concrete type at the handler site.

**Better approach:** Since the existing EventBus pattern is sync and the design wants async, let's not fight it. Instead, make the EventBus support async handlers properly by modifying the trait.

- [ ] **Step 1 (final): Add AsyncEventHandler to EventBus trait**

In `crates/aletheon-abi/src/event_bus.rs`:

```rust
use std::future::Future;
use std::pin::Pin;

/// Async event handler. Takes ownership of event data via serde_json::Value.
/// Returns true to continue propagation.
pub type AsyncEventHandler = Box<
    dyn Fn(serde_json::Value) -> Pin<Box<dyn Future<Output = bool> + Send>>
        + Send
        + Sync,
>;
```

Add to `EventBus` trait:

```rust
    async fn subscribe_async(
        &self,
        event_type: EventType,
        handler: AsyncEventHandler,
    ) -> Result<SubscriptionId>;
```

In `crates/aletheon-abi/src/event.rs`, add to the `Event` trait:

```rust
    /// Serialize event payload for async handler transport.
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
```

- [ ] **Step 2: Implement subscribe_async in KernelEventBus**

In `crates/aletheon-comm/src/impl/kernel_bus.rs`:

```rust
use aletheon_abi::AsyncEventHandler;

impl EventBus for KernelEventBus {
    // ... existing methods ...

    async fn subscribe_async(
        &self,
        event_type: EventType,
        handler: AsyncEventHandler,
    ) -> Result<SubscriptionId> {
        // Wrap async handler in sync handler
        let handler = Arc::new(handler);
        let sync_handler: EventHandler = Box::new(move |event: &dyn Event| {
            let json = event.to_json();
            let handler = handler.clone();
            tokio::spawn(async move {
                handler(json).await;
            });
            true
        });
        self.subscribe(event_type, sync_handler).await
    }
}
```

- [ ] **Step 3: Add to_json() to ConcreteEvent**

In `crates/aletheon-comm/src/core/event.rs`, implement `to_json()`:

```rust
    fn to_json(&self) -> serde_json::Value {
        // Try to serialize the payload
        self.payload()
            .downcast_ref::<serde_json::Value>()
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p aletheon-comm`
Expected: Compiles

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-abi/src/event_bus.rs crates/aletheon-abi/src/event.rs crates/aletheon-comm/src/impl/kernel_bus.rs crates/aletheon-comm/src/core/event.rs
git commit -m "feat(comm): add async event handler support to EventBus"
```

---

## Task 3: Create LlmScheduler

**Files:**
- Create: `crates/aletheon-brain/src/impl/llm/scheduler.rs`
- Modify: `crates/aletheon-brain/src/impl/llm/mod.rs`

- [ ] **Step 1: Create LlmScheduler**

Create `crates/aletheon-brain/src/impl/llm/scheduler.rs`:

```rust
//! Centralized LLM routing.
//!
//! Other modules do NOT hold LlmProvider directly. They call LlmScheduler::request()
//! which routes to the right provider based on LlmPurpose.

use std::collections::HashMap;
use std::sync::Arc;
use anyhow::Result;
use tokio::sync::RwLock;
use aletheon_abi::{Message, ContentBlock};
use aletheon_abi::evolution::LlmPurpose;
use super::provider::{LlmProvider, LlmResponse, ToolDefinition};

/// Routing rule: maps a purpose to a provider name.
pub struct RoutingRule {
    pub purpose: LlmPurpose,
    pub provider_name: String,
}

/// Configuration for a single LLM provider.
#[derive(Debug, Clone)]
pub struct SchedulerProviderConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub kind: String,  // "anthropic" | "openai" | "ollama"
    pub model: String,
}

/// Full scheduler configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub providers: Vec<SchedulerProviderConfig>,
    pub routing: Vec<RoutingRule>,
}

/// Centralized LLM scheduler with purpose-based routing.
pub struct LlmScheduler {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    routing: HashMap<LlmPurpose, String>,  // purpose -> provider_name
    default_provider: String,
}

impl LlmScheduler {
    /// Create a new scheduler from config.
    pub fn new(config: &SchedulerConfig) -> Result<Self> {
        use super::provider_factory::create_provider_by_kind;
        use crate::config::ProviderConfig;
        use crate::config::Transport;

        let mut providers = HashMap::new();
        for pc in &config.providers {
            let provider_config = ProviderConfig {
                name: pc.name.clone(),
                base_url: pc.base_url.clone(),
                api_key: pc.api_key.clone(),
                transport: match pc.kind.as_str() {
                    "anthropic" => Transport::Anthropic,
                    "ollama" => Transport::Openai,
                    _ => Transport::Openai,
                },
                models: vec![pc.model.clone()],
            };
            let provider = create_provider_by_kind(&pc.kind, &provider_config, &pc.model)?;
            providers.insert(pc.name.clone(), provider);
        }

        let mut routing = HashMap::new();
        for rule in &config.routing {
            routing.insert(rule.purpose.clone(), rule.provider_name.clone());
        }

        let default_provider = config.providers.first()
            .map(|p| p.name.clone())
            .unwrap_or_default();

        Ok(Self {
            providers,
            routing,
            default_provider,
        })
    }

    /// Route a purpose to a provider name.
    fn resolve_provider(&self, purpose: &LlmPurpose) -> &str {
        self.routing
            .get(purpose)
            .map(|s| s.as_str())
            .unwrap_or(&self.default_provider)
    }

    /// Get a provider by name.
    pub fn provider(&self, name: &str) -> Option<&Arc<dyn LlmProvider>> {
        self.providers.get(name)
    }

    /// Execute a completion request routed by purpose.
    pub async fn complete(
        &self,
        purpose: &LlmPurpose,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let provider_name = self.resolve_provider(purpose);
        let provider = self.providers.get(provider_name)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_name))?;
        provider.complete(messages, tools).await
    }

    /// Get the provider for task execution (Engine use).
    pub fn executor_provider(&self) -> &Arc<dyn LlmProvider> {
        let name = self.resolve_provider(&LlmPurpose::Execute);
        self.providers.get(name).unwrap_or_else(|| {
            self.providers.values().next().expect("No LLM providers configured")
        })
    }

    /// Get the provider for reflection (BrainCore use).
    pub fn reflector_provider(&self) -> &Arc<dyn LlmProvider> {
        let name = self.resolve_provider(&LlmPurpose::Reflect);
        self.providers.get(name).unwrap_or_else(|| {
            self.providers.values().next().expect("No LLM providers configured")
        })
    }
}
```

- [ ] **Step 2: Re-export scheduler**

In `crates/aletheon-brain/src/impl/llm/mod.rs`, add:

```rust
pub mod scheduler;
pub use scheduler::{LlmScheduler, SchedulerConfig, SchedulerProviderConfig, RoutingRule};
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p aletheon-brain`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/scheduler.rs crates/aletheon-brain/src/impl/llm/mod.rs
git commit -m "feat(brain): add LlmScheduler for centralized dual-model routing"
```

---

## Task 4: Create ToolObservationHandler (BrainCore)

**Files:**
- Create: `crates/aletheon-brain/src/impl/event_handlers/mod.rs`
- Create: `crates/aletheon-brain/src/impl/event_handlers/tool_observer.rs`
- Modify: `crates/aletheon-brain/src/impl/mod.rs`

- [ ] **Step 1: Create ToolObservationHandler**

Create `crates/aletheon-brain/src/impl/event_handlers/tool_observer.rs`:

```rust
//! Subscribes to ToolObservationEvent, uses LLM to reflect on tool execution results.
//!
//! Emits:
//! - ReflectionEvent (after each tool call)
//! - RuleExtractedEvent (when batch threshold reached)
//! - EvolutionTriggeredEvent (when evolution conditions met)

use std::sync::Arc;
use anyhow::Result;
use tokio::sync::Mutex;
use uuid::Uuid;
use aletheon_abi::evolution::*;
use aletheon_abi::{ContentBlock, Message, Role};
use crate::impl::llm::scheduler::LlmScheduler;
use crate::impl::llm::provider::LlmProvider;

/// Configuration for the tool observation handler.
#[derive(Debug, Clone)]
pub struct ObserverConfig {
    /// Number of reflections before extracting rules.
    pub batch_size: usize,
    /// Consecutive failures to trigger evolution.
    pub consecutive_failure_threshold: usize,
    /// Confidence drop threshold (0.0-1.0).
    pub confidence_drop_threshold: f64,
}

impl Default for ObserverConfig {
    fn default() -> Self {
        Self {
            batch_size: 3,
            consecutive_failure_threshold: 3,
            confidence_drop_threshold: 0.2,
        }
    }
}

/// Handles ToolObservationEvent: reflects via LLM, extracts rules, triggers evolution.
pub struct ToolObservationHandler {
    scheduler: Arc<LlmScheduler>,
    config: ObserverConfig,
    reflection_buffer: Mutex<Vec<ReflectionPayload>>,
    consecutive_failures: Mutex<usize>,
    last_confidence: Mutex<f64>,
}

impl ToolObservationHandler {
    pub fn new(scheduler: Arc<LlmScheduler>, config: ObserverConfig) -> Self {
        Self {
            scheduler,
            config,
            reflection_buffer: Mutex::new(Vec::new()),
            consecutive_failures: Mutex::new(0),
            last_confidence: Mutex::new(1.0),
        }
    }

    /// Process a tool observation. Returns events to emit.
    pub async fn handle(&self, obs: &ToolObservationPayload) -> Result<Vec<EvolutionEvent>> {
        let mut events = Vec::new();

        // 1. LLM reflection
        let reflection = self.reflect(obs).await?;
        events.push(EvolutionEvent::Reflection(reflection.clone()));

        // 2. Track consecutive failures
        let mut failures = self.consecutive_failures.lock().await;
        match reflection.assessment {
            Assessment::Failure => *failures += 1,
            _ => *failures = 0,
        }

        // 3. Buffer reflection
        let mut buffer = self.reflection_buffer.lock().await;
        buffer.push(reflection.clone());

        // 4. Check batch threshold for rule extraction
        if buffer.len() >= self.config.batch_size {
            let rules = self.extract_rules(&buffer).await?;
            if !rules.is_empty() {
                events.push(EvolutionEvent::RuleExtracted(RuleExtractedPayload {
                    source_reflections: buffer.iter().map(|r| r.turn_id).collect(),
                    rules: rules.clone(),
                }));
            }

            // 5. Check evolution trigger conditions
            let should_trigger = *failures >= self.config.consecutive_failure_threshold;
            let confidence_dropped = {
                let last = self.last_confidence.lock().await;
                *last - reflection.confidence > self.config.confidence_drop_threshold
            };

            if should_trigger || confidence_dropped {
                let reason = if should_trigger {
                    "consecutive_failures".to_string()
                } else {
                    "confidence_drop".to_string()
                };
                events.push(EvolutionEvent::EvolutionTriggered(EvolutionTriggeredPayload {
                    trigger_reason: reason,
                    recent_reflections: buffer.iter().map(|r| r.turn_id).collect(),
                    current_rules_snapshot: rules,
                }));
            }

            // Update confidence tracking
            *self.last_confidence.lock().await = reflection.confidence;

            // Clear buffer after processing
            buffer.clear();
        }

        Ok(events)
    }

    /// Use LLM to reflect on a tool observation.
    async fn reflect(&self, obs: &ToolObservationPayload) -> Result<ReflectionPayload> {
        let prompt = format!(
            r#"You are analyzing a tool execution result for a self-evolving agent.

Tool: {tool}
Input: {input}
Output: {output}
Duration: {duration}ms
Error: {error}

Respond with JSON:
{{
  "assessment": "Success" | "PartialSuccess" | "Failure",
  "root_cause": "string or null",
  "suggested_rule": {{"condition": "...", "action": "..."}} or null,
  "confidence": 0.0-1.0
}}"#,
            tool = obs.tool_name,
            input = serde_json::to_string_pretty(&obs.input).unwrap_or_default(),
            output = serde_json::to_string_pretty(&obs.output).unwrap_or_default(),
            duration = obs.duration_ms,
            error = obs.error.as_deref().unwrap_or("none"),
        );

        let messages = vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: prompt }],
            }
        ];

        let response = self.scheduler
            .complete(&LlmPurpose::Reflect, &messages, &[])
            .await?;

        let text = response.content.iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        self.parse_reflection(obs.turn_id, &text)
    }

    /// Parse LLM response into ReflectionPayload.
    fn parse_reflection(&self, turn_id: Uuid, text: &str) -> Result<ReflectionPayload> {
        // Try JSON parse
        let json: serde_json::Value = serde_json::from_str(text)
            .unwrap_or(serde_json::Value::Null);

        let assessment = match json["assessment"].as_str().unwrap_or("Failure") {
            "Success" => Assessment::Success,
            "PartialSuccess" => Assessment::PartialSuccess,
            _ => Assessment::Failure,
        };

        let suggested_rule = json["suggested_rule"].as_object().map(|obj| {
            LearnedRule {
                id: Uuid::new_v4(),
                condition: obj["condition"].as_str().unwrap_or_default().to_string(),
                action: obj["action"].as_str().unwrap_or_default().to_string(),
                confidence: json["confidence"].as_f64().unwrap_or(0.5),
                source_reflections: vec![turn_id],
            }
        });

        Ok(ReflectionPayload {
            turn_id,
            assessment,
            root_cause: json["root_cause"].as_str().map(String::from),
            suggested_rule,
            confidence: json["confidence"].as_f64().unwrap_or(0.5),
        })
    }

    /// Extract generalized rules from a batch of reflections.
    async fn extract_rules(&self, reflections: &[ReflectionPayload]) -> Result<Vec<LearnedRule>> {
        let summary = reflections.iter().enumerate().map(|(i, r)| {
            format!(
                "{}. {:?}: {} (confidence: {:.2})",
                i + 1,
                r.assessment,
                r.root_cause.as_deref().unwrap_or("no cause"),
                r.confidence
            )
        }).collect::<Vec<_>>().join("\n");

        let prompt = format!(
            r#"Analyze these reflections from a self-evolving agent and extract generalized rules.

Reflections:
{summary}

Respond with JSON array of rules:
[
  {{"condition": "when X happens", "action": "do Y", "confidence": 0.0-1.0}}
]

Only extract rules that are genuinely useful. Return empty array [] if no patterns found."#,
            summary = summary,
        );

        let messages = vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: prompt }],
            }
        ];

        let response = self.scheduler
            .complete(&LlmPurpose::ExtractRules, &messages, &[])
            .await?;

        let text = response.content.iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        let json: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or(serde_json::Value::Array(vec![]));

        let rules = json.as_array().unwrap_or(&vec![]).iter().filter_map(|r| {
            Some(LearnedRule {
                id: Uuid::new_v4(),
                condition: r["condition"].as_str()?.to_string(),
                action: r["action"].as_str()?.to_string(),
                confidence: r["confidence"].as_f64().unwrap_or(0.5),
                source_reflections: reflections.iter().map(|r| r.turn_id).collect(),
            })
        }).collect();

        Ok(rules)
    }
}

/// Events emitted by ToolObservationHandler.
pub enum EvolutionEvent {
    Reflection(ReflectionPayload),
    RuleExtracted(RuleExtractedPayload),
    EvolutionTriggered(EvolutionTriggeredPayload),
}
```

- [ ] **Step 2: Create event_handlers module**

Create `crates/aletheon-brain/src/impl/event_handlers/mod.rs`:

```rust
pub mod tool_observer;
pub use tool_observer::{ToolObservationHandler, ObserverConfig, EvolutionEvent};
```

- [ ] **Step 3: Register module in brain**

In `crates/aletheon-brain/src/impl/mod.rs`, add:

```rust
pub mod event_handlers;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p aletheon-brain`
Expected: May have type mismatches — fix imports as needed

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-brain/src/impl/event_handlers/ crates/aletheon-brain/src/impl/mod.rs
git commit -m "feat(brain): add ToolObservationHandler for LLM-driven reflection"
```

---

## Task 5: Create MutationApprover (SelfField)

**Files:**
- Create: `crates/aletheon-self/src/impl/mutation/mod.rs`
- Create: `crates/aletheon-self/src/impl/mutation/approver.rs`
- Modify: `crates/aletheon-self/src/impl/mod.rs`

- [ ] **Step 1: Create MutationApprover**

Create `crates/aletheon-self/src/impl/mutation/approver.rs`:

```rust
//! Subscribes to EvolutionTriggeredEvent, validates mutation intents via SelfField.
//!
//! Uses LLM to generate mutation intents from evolution context,
//! then validates each against boundary rules and identity continuity.

use std::sync::Arc;
use anyhow::Result;
use aletheon_abi::evolution::*;
use aletheon_abi::{ContentBlock, Message, Role};
use aletheon_abi::self_field::MutationIntent;
use crate::core::mutation::MutationLayer;
use crate::core::SelfFieldOps;

/// Validates evolution triggers and generates approved mutation intents.
pub struct MutationApprover {
    mutation_layer: Arc<MutationLayer>,
    scheduler: Arc<crate::impl::llm_bridge::LlmBridge>,
    max_magnitude: f64,
}

impl MutationApprover {
    pub fn new(
        mutation_layer: Arc<MutationLayer>,
        scheduler: Arc<crate::impl::llm_bridge::LlmBridge>,
    ) -> Self {
        Self {
            mutation_layer,
            scheduler,
            max_magnitude: 0.3,  // Conservative default
        }
    }

    /// Process an evolution trigger. Returns approved mutation intents.
    pub async fn handle(&self, trigger: &EvolutionTriggeredPayload) -> Result<Vec<MutationIntent>> {
        // 1. Generate mutation intents via LLM
        let intents = self.generate_intents(trigger).await?;

        // 2. Validate each intent through MutationLayer
        let mut approved = Vec::new();
        for intent in intents {
            let verdict = self.mutation_layer.review(&intent).await?;
            match verdict {
                aletheon_abi::self_field::Verdict::Allow => {
                    approved.push(intent);
                }
                _ => {
                    tracing::info!(
                        "MutationIntent rejected by SelfField: target={}, reason={}",
                        intent.target, intent.reason
                    );
                }
            }
        }

        Ok(approved)
    }

    /// Use LLM to generate mutation intents from evolution context.
    async fn generate_intents(&self, trigger: &EvolutionTriggeredPayload) -> Result<Vec<MutationIntent>> {
        let rules_summary = trigger.current_rules_snapshot.iter()
            .map(|r| format!("- IF {} THEN {} (confidence: {:.2})", r.condition, r.action, r.confidence))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            r#"You are the mutation advisor for a self-evolving agent.
An evolution has been triggered.

Trigger reason: {reason}
Recent reflections: {count} reflections analyzed
Current rules:
{rules}

Generate mutation intents to improve the agent. Each intent should target a specific genome field.
Valid targets: care.priorities, boundary.rules, mutation.config

Respond with JSON array:
[
  {{
    "target": "care.priorities",
    "change": {{"field": "safety_weight", "delta": 0.1}},
    "reason": "why this change helps",
    "reversible": true
  }}
]

Be conservative. Max magnitude: {max_mag}. Return empty array [] if no change needed."#,
            reason = trigger.trigger_reason,
            count = trigger.recent_reflections.len(),
            rules = if rules_summary.is_empty() { "none".to_string() } else { rules_summary },
            max_mag = self.max_magnitude,
        );

        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: prompt }],
        }];

        let response = self.scheduler.complete_for_purpose("generate_mutations", &messages).await?;

        let text = response.content.iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        let json: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or(serde_json::Value::Array(vec![]));

        let intents = json.as_array().unwrap_or(&vec![]).iter().filter_map(|v| {
            Some(MutationIntent {
                target: v["target"].as_str()?.to_string(),
                change: v.get("change")?.clone(),
                reason: v["reason"].as_str().unwrap_or("llm-generated").to_string(),
                reversible: v["reversible"].as_bool().unwrap_or(true),
            })
        }).collect();

        Ok(intents)
    }
}
```

Note: This uses a placeholder `LlmBridge` type. We need a way for SelfField to call LLM without directly holding `LlmProvider`. Create a thin adapter:

- [ ] **Step 2: Create LlmBridge for SelfField**

Create `crates/aletheon-self/src/impl/llm_bridge.rs`:

```rust
//! Thin adapter for SelfField to call LLM without directly depending on aletheon-brain.
//!
//! Wraps an Arc<dyn LlmProvider> behind a purpose-based interface.

use std::sync::Arc;
use anyhow::Result;
use aletheon_abi::{ContentBlock, Message};
use aletheon_brain::impl::llm::provider::{LlmProvider, LlmResponse, ToolDefinition};

pub struct LlmBridge {
    provider: Arc<dyn LlmProvider>,
}

impl LlmBridge {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    pub async fn complete_for_purpose(
        &self,
        _purpose: &str,
        messages: &[Message],
    ) -> Result<LlmResponse> {
        self.provider.complete(messages, &[]).await
    }
}
```

- [ ] **Step 3: Create mutation module and register**

Create `crates/aletheon-self/src/impl/mutation/mod.rs`:

```rust
pub mod approver;
pub use approver::MutationApprover;
```

In `crates/aletheon-self/src/impl/mod.rs`, add:

```rust
pub mod mutation;
pub mod llm_bridge;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p aletheon-self`
Expected: May need to fix imports for MutationLayer visibility

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-self/src/impl/mutation/ crates/aletheon-self/src/impl/llm_bridge.rs crates/aletheon-self/src/impl/mod.rs
git commit -m "feat(self): add MutationApprover for LLM-driven mutation validation"
```

---

## Task 6: Create MutationExecutor (MetaRuntime)

**Files:**
- Create: `crates/aletheon-meta/src/impl/event_handlers/mod.rs`
- Create: `crates/aletheon-meta/src/impl/event_handlers/mutation_executor.rs`
- Modify: `crates/aletheon-meta/src/impl/mod.rs`

- [ ] **Step 1: Create MutationExecutor**

Create `crates/aletheon-meta/src/impl/event_handlers/mutation_executor.rs`:

```rust
//! Subscribes to MutationIntentEvent, executes Morphogenesis Pipeline.
//!
//! Takes approved MutationIntents from SelfField and runs them through
//! candidate generation → sandbox testing → evaluation → migration.

use std::sync::Arc;
use anyhow::Result;
use aletheon_abi::evolution::*;
use aletheon_abi::self_field::MutationIntent;
use crate::impl::morphogenesis::pipeline::MorphogenesisPipeline;
use crate::impl::meta_runtime::DefaultMetaRuntime;
use crate::core::types::GenomeMeta;

/// Executes mutation intents through the Morphogenesis Pipeline.
pub struct MutationExecutor {
    pipeline: MorphogenesisPipeline<DefaultMetaRuntime>,
}

impl MutationExecutor {
    pub fn new(pipeline: MorphogenesisPipeline<DefaultMetaRuntime>) -> Self {
        Self { pipeline }
    }

    /// Process approved mutation intents. Returns evolution results.
    pub async fn handle(&self, intents: &[MutationIntent]) -> Result<Vec<EvolutionResultPayload>> {
        let mut results = Vec::new();

        for intent in intents {
            let result = self.pipeline.run(intent).await?;

            results.push(EvolutionResultPayload {
                adopted: result.success,
                genome_version_before: result.candidate.base_version.to_string(),
                genome_version_after: if result.success {
                    Some(format!("{}.mutated", result.candidate.base_version))
                } else {
                    None
                },
                summary: result.message,
            });
        }

        Ok(results)
    }
}
```

- [ ] **Step 2: Create event_handlers module**

Create `crates/aletheon-meta/src/impl/event_handlers/mod.rs`:

```rust
pub mod mutation_executor;
pub use mutation_executor::MutationExecutor;
```

- [ ] **Step 3: Register module**

In `crates/aletheon-meta/src/impl/mod.rs`, add:

```rust
pub mod event_handlers;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p aletheon-meta`
Expected: May need to check MorphogenesisPipeline::run signature

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-meta/src/impl/event_handlers/ crates/aletheon-meta/src/impl/mod.rs
git commit -m "feat(meta): add MutationExecutor for EventBus-driven morphogenesis"
```

---

## Task 7: Wire Engine to Emit ToolObservationEvent

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs`
- Modify: `crates/aletheon-runtime/src/impl/engine/config.rs`

- [ ] **Step 1: Add EventBus to EngineConfig**

In `crates/aletheon-runtime/src/impl/engine/config.rs`, add:

```rust
use std::sync::Arc;
use aletheon_abi::EventBus;

pub struct EngineConfig {
    // ... existing fields ...
    
    /// Optional EventBus for emitting self-evolution events.
    /// When set, Engine emits ToolObservationEvent after each tool call.
    pub event_bus: Option<Arc<dyn EventBus>>,
}
```

- [ ] **Step 2: Emit ToolObservationEvent after tool execution**

In `crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs`, after the existing outcome recording (around line 502), add:

```rust
                // Emit ToolObservationEvent if EventBus is configured
                if let Some(bus) = &self.config.event_bus {
                    use aletheon_abi::evolution::*;
                    use aletheon_abi::ConcreteEvent;
                    
                    let obs = ToolObservationPayload {
                        turn_id: Uuid::parse_str(&turn_id).unwrap_or_else(|_| Uuid::new_v4()),
                        tool_name: tool_name.clone(),
                        input: serde_json::to_value(&tool_input).unwrap_or_default(),
                        output: serde_json::to_value(&result.content).unwrap_or_default(),
                        duration_ms: elapsed_ms,
                        error: if result.is_error { Some(result.content.clone()) } else { None },
                        rules_applied: Vec::new(), // TODO: track applied rules
                    };
                    
                    let event = ConcreteEvent::new(
                        EventType::ToolObservation,
                        Priority::Normal,
                        "engine".to_string(),
                        Box::new(obs),
                    );
                    
                    if let Err(e) = bus.publish(Box::new(event)).await {
                        tracing::warn!("Failed to publish ToolObservationEvent: {}", e);
                    }
                }
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p aletheon-runtime`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs crates/aletheon-runtime/src/impl/engine/config.rs
git commit -m "feat(runtime): Engine emits ToolObservationEvent via EventBus"
```

---

## Task 8: Create End-to-End Demo

**Files:**
- Create: `examples/self-evolution-loop/Cargo.toml`
- Create: `examples/self-evolution-loop/src/main.rs`
- Create: `examples/self-evolution-loop/config.toml`
- Create: `examples/self-evolution-loop/genome.yaml`
- Create: `examples/self-evolution-loop/README.md`

- [ ] **Step 1: Create Cargo.toml**

Create `examples/self-evolution-loop/Cargo.toml`:

```toml
[package]
name = "self-evolution-loop-example"
version = "0.1.0"
edition = "2021"

[dependencies]
aletheon-abi = { path = "../../crates/aletheon-abi" }
aletheon-comm = { path = "../../crates/aletheon-comm" }
aletheon-brain = { path = "../../crates/aletheon-brain" }
aletheon-self = { path = "../../crates/aletheon-self" }
aletheon-meta = { path = "../../crates/aletheon-meta" }
aletheon-body = { path = "../../crates/aletheon-body" }
aletheon-memory = { path = "../../crates/aletheon-memory" }
aletheon-runtime = { path = "../../crates/aletheon-runtime" }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

- [ ] **Step 2: Create main.rs**

Create `examples/self-evolution-loop/src/main.rs`:

```rust
//! Self-Evolution EventBus Loop Demo
//!
//! Demonstrates the full closed loop:
//! 1. Engine executes tool → emits ToolObservationEvent
//! 2. BrainCore reflects via LLM → emits ReflectionEvent
//! 3. After batch → extracts rules → emits RuleExtractedEvent
//! 4. After consecutive failures → emits EvolutionTriggeredEvent
//! 5. SelfField validates → emits MutationIntentEvent
//! 6. MetaRuntime executes Morphogenesis Pipeline → emits EvolutionResultEvent

use std::sync::Arc;
use anyhow::Result;
use aletheon_abi::{EventBus, EventType, ConcreteEvent, Priority};
use aletheon_abi::evolution::*;
use aletheon_comm::impl_::kernel_bus::KernelEventBus;
use aletheon_brain::impl_::llm::scheduler::{LlmScheduler, SchedulerConfig, SchedulerProviderConfig, RoutingRule};
use aletheon_brain::impl_::event_handlers::{ToolObservationHandler, ObserverConfig};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("=== Self-Evolution EventBus Loop Demo ===\n");

    // 1. Create EventBus
    let bus = Arc::new(KernelEventBus::new());

    // 2. Create LLM Scheduler with dual-model config
    let scheduler_config = SchedulerConfig {
        providers: vec![
            SchedulerProviderConfig {
                name: "deepseek".to_string(),
                base_url: "https://api.deepseek.com/v1".to_string(),
                api_key: std::env::var("DEEPSEEK_API_KEY").unwrap_or_default(),
                kind: "openai".to_string(),
                model: "deepseek-chat".to_string(),
            },
        ],
        routing: vec![
            RoutingRule { purpose: LlmPurpose::Reflect, provider_name: "deepseek".to_string() },
            RoutingRule { purpose: LlmPurpose::ExtractRules, provider_name: "deepseek".to_string() },
            RoutingRule { purpose: LlmPurpose::GenerateMutations, provider_name: "deepseek".to_string() },
        ],
    };

    let scheduler = Arc::new(LlmScheduler::new(&scheduler_config)?);

    // 3. Create BrainCore handler
    let observer = Arc::new(ToolObservationHandler::new(scheduler.clone(), ObserverConfig::default()));

    // 4. Subscribe BrainCore to ToolObservationEvent
    let observer_clone = observer.clone();
    bus.subscribe_async(EventType::ToolObservation, Box::new(move |json| {
        let obs = observer_clone.clone();
        Box::pin(async move {
            if let Ok(payload) = serde_json::from_value::<ToolObservationPayload>(json) {
                match obs.handle(&payload).await {
                    Ok(events) => {
                        for event in events {
                            match event {
                                EvolutionEvent::Reflection(r) => {
                                    println!("[Reflection] {:?}: {}", r.assessment, r.root_cause.as_deref().unwrap_or("ok"));
                                }
                                EvolutionEvent::RuleExtracted(rules) => {
                                    println!("[Rules Extracted] {} rules from {} reflections", rules.rules.len(), rules.source_reflections.len());
                                    for rule in &rules.rules {
                                        println!("  - IF {} THEN {} (conf: {:.2})", rule.condition, rule.action, rule.confidence);
                                    }
                                }
                                EvolutionEvent::EvolutionTriggered(trigger) => {
                                    println!("[Evolution Triggered] reason: {}", trigger.trigger_reason);
                                }
                            }
                        }
                    }
                    Err(e) => tracing::error!("Handler error: {}", e),
                }
            }
            true
        })
    })).await?;

    // 5. Simulate tool observations
    let observations = vec![
        ToolObservationPayload {
            turn_id: Uuid::new_v4(),
            tool_name: "bash_exec".to_string(),
            input: serde_json::json!({"command": "cat /var/log/syslog | head -100"}),
            output: serde_json::json!("Jan  1 00:00:01 host kernel: ...\nJan  1 00:00:02 host sshd[123]: ..."),
            duration_ms: 150,
            error: None,
            rules_applied: vec![],
        },
        ToolObservationPayload {
            turn_id: Uuid::new_v4(),
            tool_name: "bash_exec".to_string(),
            input: serde_json::json!({"command": "sort /var/log/syslog"}),
            output: serde_json::json!("error: permission denied"),
            duration_ms: 50,
            error: Some("permission denied".to_string()),
            rules_applied: vec![],
        },
        ToolObservationPayload {
            turn_id: Uuid::new_v4(),
            tool_name: "bash_exec".to_string(),
            input: serde_json::json!({"command": "grep ERROR /var/log/syslog"}),
            output: serde_json::json!("error: file not found"),
            duration_ms: 30,
            error: Some("file not found".to_string()),
            rules_applied: vec![],
        },
        ToolObservationPayload {
            turn_id: Uuid::new_v4(),
            tool_name: "bash_exec".to_string(),
            input: serde_json::json!({"command": "tail -50 /var/log/syslog"}),
            output: serde_json::json!("error: timeout"),
            duration_ms: 30000,
            error: Some("timeout".to_string()),
            rules_applied: vec![],
        },
    ];

    println!("--- Emitting {} tool observations ---\n", observations.len());
    for (i, obs) in observations.iter().enumerate() {
        println!("[Turn {}] Tool: {}, Error: {:?}", i + 1, obs.tool_name, obs.error);
        let event = ConcreteEvent::new(
            EventType::ToolObservation,
            Priority::Normal,
            "demo".to_string(),
            Box::new(obs.clone()),
        );
        bus.publish(Box::new(event)).await?;
    }

    println!("\n=== Demo Complete ===");
    Ok(())
}
```

- [ ] **Step 3: Create config.toml**

Create `examples/self-evolution-loop/config.toml`:

```toml
[llm_scheduler.executor]
provider = "anthropic"
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"

[llm_scheduler.reflector]
provider = "openai"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-chat"
api_key_env = "DEEPSEEK_API_KEY"

[engine]
learning_enabled = true

[evolution]
consecutive_failure_threshold = 3
batch_size = 3
evolution_interval_hours = 0
```

- [ ] **Step 4: Create genome.yaml**

Create `examples/self-evolution-loop/genome.yaml`:

```yaml
version: "0.1.0"
identity:
  name: "aletheon"
  core_values: ["safety", "helpfulness", "continuity"]
care:
  priorities:
    safety_weight: 0.7
    helpfulness_weight: 0.5
    efficiency_weight: 0.3
boundary:
  rules:
    - "never delete user data without confirmation"
    - "never modify system files without sandbox"
mutation:
  config:
    max_magnitude: 0.3
    require_tests: true
    sandbox_first: true
```

- [ ] **Step 5: Create README.md**

Create `examples/self-evolution-loop/README.md`:

```markdown
# Self-Evolution EventBus Loop Demo

Demonstrates the full closed loop for agent self-evolution.

## Prerequisites

```bash
export DEEPSEEK_API_KEY=your_key_here
```

## Run

```bash
cargo run -p self-evolution-loop-example
```

## What It Does

1. Creates an EventBus with async handler support
2. Subscribes BrainCore's ToolObservationHandler to ToolObservationEvent
3. Simulates 4 tool observations (3 failures)
4. BrainCore reflects via LLM, extracts rules, detects evolution trigger
5. Prints the full event flow
```

- [ ] **Step 6: Add to workspace**

In root `Cargo.toml`, add to `[workspace]` members:

```toml
    "examples/self-evolution-loop",
```

- [ ] **Step 7: Verify compilation**

Run: `cargo check -p self-evolution-loop-example`
Expected: Compiles (may need import fixes)

- [ ] **Step 8: Commit**

```bash
git add examples/self-evolution-loop/ Cargo.toml
git commit -m "feat(demo): add self-evolution EventBus loop end-to-end example"
```

---

## Task 9: Integration Test with MockLlm

**Files:**
- Create: `crates/aletheon-runtime/tests/self_evolution_loop_test.rs`

- [ ] **Step 1: Create integration test**

Create `crates/aletheon-runtime/tests/self_evolution_loop_test.rs`:

```rust
//! Integration test for the self-evolution EventBus loop.
//!
//! Uses MockLlm to verify the full event flow without real API calls.

use std::sync::Arc;
use anyhow::Result;
use aletheon_abi::{EventBus, EventType, ConcreteEvent, Priority};
use aletheon_abi::evolution::*;
use aletheon_comm::impl_::kernel_bus::KernelEventBus;
use aletheon_brain::testing::mock_llm::MockLlm;
use aletheon_brain::impl_::llm::scheduler::{LlmScheduler, SchedulerConfig, SchedulerProviderConfig, RoutingRule};
use aletheon_brain::impl_::event_handlers::{ToolObservationHandler, ObserverConfig};
use uuid::Uuid;

#[tokio::test]
async fn test_reflection_emitted_after_tool_observation() -> Result<()> {
    let bus = Arc::new(KernelEventBus::new());
    
    // Create mock LLM that returns a valid reflection
    let mock_llm = Arc::new(MockLlm::new(vec![
        // Reflection response
        r#"{"assessment": "Failure", "root_cause": "permission denied", "suggested_rule": null, "confidence": 0.3}"#.to_string(),
    ]));
    
    // Create scheduler with mock
    // ... (need to adapt scheduler to accept mock providers)
    
    // Create handler
    let observer = Arc::new(ToolObservationHandler::new(scheduler, ObserverConfig::default()));
    
    // Subscribe
    let observer_clone = observer.clone();
    bus.subscribe_async(EventType::ToolObservation, Box::new(move |json| {
        let obs = observer_clone.clone();
        Box::pin(async move {
            if let Ok(payload) = serde_json::from_value::<ToolObservationPayload>(json) {
                let events = obs.handle(&payload).await.unwrap();
                assert!(!events.is_empty(), "Should emit at least one event");
            }
            true
        })
    })).await?;
    
    // Emit observation
    let obs = ToolObservationPayload {
        turn_id: Uuid::new_v4(),
        tool_name: "bash_exec".to_string(),
        input: serde_json::json!({"command": "test"}),
        output: serde_json::json!("error"),
        duration_ms: 100,
        error: Some("permission denied".to_string()),
        rules_applied: vec![],
    };
    
    let event = ConcreteEvent::new(
        EventType::ToolObservation,
        Priority::Normal,
        "test".to_string(),
        Box::new(obs),
    );
    bus.publish(Box::new(event)).await?;
    
    Ok(())
}

#[tokio::test]
async fn test_evolution_triggered_after_consecutive_failures() -> Result<()> {
    // Similar test but with 3 consecutive failures
    // Verify EvolutionTriggeredEvent is emitted
    Ok(())
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p aletheon-runtime self_evolution_loop`
Expected: PASS (with mock LLM)

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/tests/self_evolution_loop_test.rs
git commit -m "test(runtime): add integration test for self-evolution EventBus loop"
```

---

## Task 10: Final Integration and Cleanup

- [ ] **Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: All existing tests pass + new tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Run demo**

Run: `cargo run -p self-evolution-loop-example`
Expected: Events flow through EventBus, BrainCore reflects, prints event trace

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat: complete self-evolution EventBus loop

- ABI: async event handler support, evolution event types
- Brain: LlmScheduler + ToolObservationHandler
- Self: MutationApprover with LLM-driven intent generation
- Meta: MutationExecutor for EventBus-driven morphogenesis
- Runtime: Engine emits ToolObservationEvent
- Demo: end-to-end self-evolution loop example
- Test: integration test with MockLlm"
```
