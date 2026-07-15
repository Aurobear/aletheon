//! SelfField — the main struct wiring all 8 internal layers.
//!
//! Implements `SelfFieldOps` (the policy engine trait) and `Subsystem`
//! (the lifecycle trait). The `review()` pipeline runs:
//! Boundary -> Care -> Permissions -> Narrative -> Verdict.

pub mod attention;
pub mod awareness_growth;
pub mod boundary;
pub mod care;
pub mod conflict;
pub mod continuity;
pub mod evolution_validator;
pub mod identity;
pub mod mutation;
pub mod narrative;
pub mod store;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Duration;
use fabric::self_field::AwarenessRiskLevel;
use fabric::{
    Care, Conflict, Context, Identity, Intent, MutationIntent, Resolution, Subsystem,
    SubsystemContext, SubsystemHealth, Verdict, Version,
};
use std::sync::Arc;
use tracing::info;

use crate::bridge::hook::HookBridge;
use crate::bridge::loop_detector::LoopBridge;
use crate::bridge::policy::PolicyBridge;
use crate::core::attention::AttentionLayer;
use crate::core::boundary::{BoundaryLayer, BoundaryRule};
use crate::core::care::CareLayer;
use crate::core::conflict::ConflictLayer;
use crate::core::continuity::ContinuityLayer;
use crate::core::identity::IdentityLayer;
use crate::core::mutation::MutationLayer;
use crate::core::narrative::NarrativeLayer;

use crate::core::store::SelfFieldStore;
use crate::dasein::DaseinEventBridge;
use crate::dasein::DaseinModule;
use fabric::dasein::DaseinEvent;

/// Configuration for SelfField construction.
pub struct SelfFieldConfig {
    pub agent_name: String,
    pub agent_description: String,
    pub agent_version: String,
    pub boundary_rules: Vec<BoundaryRule>,
    pub narrative_capacity: usize,
    pub attention_decay_rate: f64,
    pub continuity_max_gap: Duration,
    /// Optional path for SQLite persistence. If None, no persistence is used.
    pub db_path: Option<std::path::PathBuf>,
    /// Enable the DaseinModule (existential substrate).
    pub enable_dasein: bool,
    /// Retention depth for the DaseinModule's temporal stream.
    pub dasein_retention_depth: usize,
    /// Decay rate for the DaseinModule's retention field (0.0-1.0).
    pub dasein_decay_rate: f64,
    /// Clock supplied by the application composition root.
    pub clock: Option<Arc<dyn fabric::Clock>>,
}

impl Default for SelfFieldConfig {
    fn default() -> Self {
        Self {
            agent_name: "aletheon".to_string(),
            agent_description: "Aletheon persistent self-evolving runtime".to_string(),
            agent_version: "0.1.0".to_string(),
            boundary_rules: Vec::new(),
            narrative_capacity: 1000,
            attention_decay_rate: 0.001,
            continuity_max_gap: Duration::hours(24),
            db_path: None,
            enable_dasein: true,
            dasein_retention_depth: 50,
            dasein_decay_rate: 0.8,
            clock: None,
        }
    }
}

/// SelfField — the policy engine implementing `SelfFieldOps`.
pub struct SelfField {
    boundary: BoundaryLayer,
    identity: IdentityLayer,
    care: CareLayer,
    narrative: NarrativeLayer,
    conflict: ConflictLayer,
    attention: AttentionLayer,
    continuity: ContinuityLayer,
    mutation: MutationLayer,
    initialized: bool,
    /// Optional SQLite store for persistence.
    store: Option<Arc<SelfFieldStore>>,
    // Bridges to external subsystems
    policy_bridge: PolicyBridge,
    loop_bridge: LoopBridge,
    hook_bridge: HookBridge,
    // DaseinModule (optional, opt-in via config)
    dasein: Option<DaseinModule>,
    dasein_event_tx: Option<tokio::sync::mpsc::Sender<DaseinEvent>>,
    /// Optional Runtime permission authority. When set, `review()` delegates
    /// the confirmation verdict to it instead of using the inline rule.
    permission_authority:
        Option<Arc<dyn fabric::policy::permission_authority::PermissionAuthority>>,
    /// Clock for deterministic time in sub-modules.
    #[allow(dead_code)]
    clock: Arc<dyn fabric::Clock>,
}

impl SelfField {
    pub fn new(config: SelfFieldConfig) -> Self {
        let clock: Arc<dyn fabric::Clock> = config
            .clock
            .expect("SelfFieldConfig.clock must be injected by the composition root");

        let mut boundary = BoundaryLayer::new();
        boundary.set_rules(config.boundary_rules);

        let identity = IdentityLayer::new(
            &config.agent_name,
            &config.agent_description,
            &config.agent_version,
            clock.clone(),
        );

        let narrative = NarrativeLayer::new(config.narrative_capacity, clock.clone());
        let attention = AttentionLayer::new(config.attention_decay_rate, clock.clone());
        let continuity = ContinuityLayer::new(config.continuity_max_gap, clock.clone());

        // Record initial identity in continuity
        continuity.record(&config.agent_name, &config.agent_version, "initialized");

        let store = config
            .db_path
            .and_then(|path| SelfFieldStore::new(path).ok())
            .map(Arc::new);

        let (dasein, dasein_event_tx) = if config.enable_dasein {
            let runtime_config = crate::dasein::DaseinRuntimeConfig {
                retention_depth: config.dasein_retention_depth,
                decay_rate: config.dasein_decay_rate,
                ..Default::default()
            };
            let ledger = store
                .as_ref()
                .map(|store| Arc::new(crate::dasein::ledger::SelfLedger::new(store.clone())));
            let (module, tx) = DaseinModule::with_runtime_and_ledger(
                clock.clone(),
                Arc::new(crate::dasein::sorge::SystemSorgeTimer),
                runtime_config,
                ledger,
            )
            .expect("SelfField Dasein configuration must be valid");
            (Some(module), Some(tx))
        } else {
            (None, None)
        };

        Self {
            boundary,
            identity,
            care: CareLayer::new(),
            narrative,
            conflict: ConflictLayer::new(),
            attention,
            continuity,
            mutation: MutationLayer::new(clock.clone()),
            initialized: false,
            store,
            policy_bridge: PolicyBridge::new(),
            loop_bridge: LoopBridge::new(),
            hook_bridge: HookBridge::new(clock.clone()),
            dasein,
            permission_authority: None,
            dasein_event_tx,
            clock,
        }
    }

    /// Access the boundary layer (for configuring rules at runtime).
    pub fn boundary_mut(&mut self) -> &mut BoundaryLayer {
        &mut self.boundary
    }

    /// Access the boundary layer (immutable).
    pub fn boundary(&self) -> &BoundaryLayer {
        &self.boundary
    }

    /// Install the Runtime's permission authority.
    ///
    /// Without it, the inline fallback rule at `review()` lines 389-398
    /// is used (behavior-preserving). This is called by the Runtime
    /// daemon handler after constructing SelfField.
    pub fn set_permission_authority(
        &mut self,
        authority: Arc<dyn fabric::policy::permission_authority::PermissionAuthority>,
    ) {
        self.permission_authority = Some(authority);
    }

    /// Access the care layer.
    pub fn care(&self) -> &CareLayer {
        &self.care
    }

    /// Access the care layer (mutable).
    pub fn care_mut(&mut self) -> &CareLayer {
        // CareLayer uses interior mutability (RwLock), so &CareLayer suffices for mutation.
        // This method exists for API clarity when the caller intends to modify.
        &self.care
    }

    /// Access the narrative layer.
    pub fn narrative(&self) -> &NarrativeLayer {
        &self.narrative
    }

    /// Access the attention layer.
    pub fn attention(&self) -> &AttentionLayer {
        &self.attention
    }

    /// Check for loops (called by Runtime during ReAct loop).
    pub fn check_loops(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        turn_id: &str,
    ) -> Option<Verdict> {
        self.loop_bridge.pre_check(tool_name, args, turn_id)
    }

    /// Record a completed tool call for loop detection.
    pub fn record_tool_result(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        result: &fabric::tool::ToolResult,
        turn_id: &str,
    ) {
        self.loop_bridge
            .post_check(tool_name, args, result, turn_id);
    }

    /// Notify new turn for loop detection.
    pub fn on_new_turn(&self, turn_id: &str) {
        self.loop_bridge.on_new_turn(turn_id);
    }

    /// End turn for loop detection.
    pub fn end_turn(&self, turn_id: &str) {
        self.loop_bridge.end_turn(turn_id);
    }

    /// Access the DaseinModule if enabled.
    pub fn dasein(&self) -> Option<&DaseinModule> {
        self.dasein.as_ref()
    }

    /// Get DaseinContext for LLM injection.
    pub fn dasein_context(&self) -> Option<fabric::dasein::DaseinContext> {
        self.dasein.as_ref().map(|d| d.to_context_injection())
    }

    /// Get formatted Dasein context for prompt injection.
    pub fn dasein_prompt_injection(&self) -> Option<String> {
        self.dasein.as_ref().map(|d| d.format_context())
    }

    /// Get the DaseinModule event sender, if Dasein is enabled.
    pub fn dasein_event_tx(&self) -> Option<&tokio::sync::mpsc::Sender<DaseinEvent>> {
        self.dasein_event_tx.as_ref()
    }

    /// Connect the DaseinModule to the CommunicationBus for real system event integration.
    ///
    /// This should be called after SelfField::init() when the CommunicationBus is available.
    /// It subscribes the DaseinModule to tool execution, memory, evolution, and
    /// session lifecycle events.
    pub async fn wire_dasein_event_bridge(
        &self,
        communication_bus: &fabric::CommunicationBus,
    ) -> anyhow::Result<()> {
        if let (Some(ref _dasein), Some(ref tx)) = (&self.dasein, &self.dasein_event_tx) {
            let bridge = DaseinEventBridge::new(tx.clone());
            bridge.subscribe(communication_bus).await?;
            info!("DaseinModule connected to EventBus via DaseinEventBridge");
        }
        Ok(())
    }

    /// Persist all layer states to the SQLite store (no-op if no store).
    pub fn save_all(&self) -> Result<()> {
        if let Some(ref store) = self.store {
            self.narrative.save_to_store(store)?;
            self.attention.save_to_store(store)?;
            self.care.save_to_store(store)?;
            self.boundary.save_to_store(store)?;
            self.identity.save_to_store(store)?;
            self.mutation.save_to_store(store)?;
            self.continuity.save_to_store(store)?;
            info!("SelfField: all layers persisted to store");
        }
        Ok(())
    }

    /// Load all layer states from the SQLite store (no-op if no store).
    pub fn load_all(&mut self) -> Result<()> {
        if let Some(ref store) = self.store {
            self.narrative.load_from_store(store)?;
            self.attention.load_from_store(store)?;
            self.care.load_from_store(store)?;
            self.boundary.load_from_store(store)?;
            self.identity.load_from_store(store)?;
            self.mutation.load_from_store(store)?;
            self.continuity.load_from_store(store)?;
            info!("SelfField: all layers loaded from store");
        }
        Ok(())
    }
}

#[async_trait]
impl Subsystem for SelfField {
    fn name(&self) -> &str {
        "self_field"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        info!("SelfField initializing");
        self.load_all()?;
        self.narrative
            .narrate("init", "SelfField subsystem initialized");
        if let Some(ref dasein) = self.dasein {
            if let Some(ref store) = self.store {
                crate::dasein::persistence::load_dasein_state(dasein, store).await?;
            }
            dasein.start_sorge_loop();
            info!("DaseinModule durable state restored; sorge loop started");
        }
        self.initialized = true;
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        if !self.initialized {
            return SubsystemHealth::Degraded {
                reason: "Not yet initialized".to_string(),
            };
        }
        SubsystemHealth::Healthy
    }

    async fn shutdown(&mut self) -> Result<()> {
        info!("SelfField shutting down");
        if let Some(ref dasein) = self.dasein {
            dasein.stop_sorge_loop().await;
            info!("DaseinModule sorge loop stopped");
            if let Some(ref store) = self.store {
                crate::dasein::persistence::save_dasein_state(dasein, store)?;
            }
        }
        self.narrative
            .narrate("shutdown", "SelfField subsystem shutting down");
        self.save_all()?;
        self.initialized = false;
        Ok(())
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}

#[async_trait]
impl fabric::SelfFieldOps for SelfField {
    /// Core review pipeline: Hook -> Policy -> Boundary -> Care -> Permissions -> Narrative -> Verdict.
    async fn review(&self, intent: &Intent, ctx: &Context) -> Result<Verdict> {
        // 1. Hook check (pre-tool hooks can block)
        if let Some(verdict) = self
            .hook_bridge
            .fire_pre_tool(&intent.action, &intent.parameters.to_string())
            .await
        {
            self.narrative.record(
                "hook_check",
                &format!("Blocked by hook: {:?}", verdict),
                Some(&intent.action),
                &verdict,
            );
            return Ok(verdict);
        }

        // 2. Policy check (PolicyEngine)
        if let Some(verdict) = self.policy_bridge.check(&intent.action, &intent.parameters) {
            self.narrative.record(
                "policy_check",
                &format!("Blocked by policy: {:?}", verdict),
                Some(&intent.action),
                &verdict,
            );
            return Ok(verdict);
        }

        // 3. Boundary check (fast gate)
        if let Some(verdict) = self.boundary.check(intent) {
            self.narrative.record(
                "boundary_check",
                &format!("Boundary rule matched: {:?}", verdict),
                Some(&intent.action),
                &verdict,
            );
            return Ok(verdict);
        }

        // 4. Care scoring
        let care_score = self.care.score_action(&intent.description);

        // 5. Permission check -- delegate to the Runtime authority if installed,
        //    otherwise fall back to the historical inline rule (behavior-preserving).
        if let Some(authority) = &self.permission_authority {
            if let Some(verdict) = authority.confirmation_verdict(ctx, care_score, &intent.action) {
                self.narrative.record(
                    "permission_check",
                    "Runtime permission authority required confirmation",
                    Some(&intent.action),
                    &verdict,
                );
                return Ok(verdict);
            }
        } else if care_score > 0.8 {
            // Fallback: historical inline rule (exact port, line-for-line).
            if ctx.permissions.max_level() < fabric::CapabilityLevel::SystemChange {
                let verdict = Verdict::RequireConfirmation {
                    reason: format!(
                        "High care relevance ({:.2}) with insufficient permissions for action '{}'",
                        care_score, intent.action
                    ),
                    risk_level: AwarenessRiskLevel::Medium,
                };
                self.narrative.record(
                    "permission_check",
                    "Insufficient permissions for high-care action",
                    Some(&intent.action),
                    &verdict,
                );
                return Ok(verdict);
            }
        }

        // 6. Record decision
        let verdict = Verdict::Allow;
        self.narrative.record(
            "review",
            &format!("Allowed: care_score={:.2}", care_score),
            Some(&intent.action),
            &verdict,
        );

        // 7. Track attention
        self.attention.attend(&intent.action, care_score);

        Ok(verdict)
    }

    async fn identity(&self) -> Result<Identity> {
        Ok(self.identity.current())
    }

    async fn cares(&self) -> Result<Vec<Care>> {
        Ok(self.care.all_cares())
    }

    async fn narrate(&self, event: &str, reason: &str) -> Result<()> {
        self.narrative.narrate(event, reason);
        Ok(())
    }

    async fn resolve_conflict(&self, conflict: &Conflict) -> Result<Resolution> {
        let resolution = self.conflict.resolve(conflict);
        self.narrative.record(
            "conflict_resolution",
            &format!("Resolved: {:?}", resolution),
            None,
            &resolution,
        );
        Ok(resolution)
    }

    async fn review_mutation(&self, mutation: &MutationIntent) -> Result<Verdict> {
        let verdict = self.mutation.review(mutation);
        self.narrative.record(
            "mutation_review",
            &format!("Mutation '{}' -> {:?}", mutation.target, verdict),
            None,
            &verdict,
        );
        Ok(verdict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::self_field::{AwarenessRiskLevel, ConflictSource};
    use fabric::{IntentSource, SelfFieldOps};
    use serde_json::json;
    use std::path::PathBuf;

    fn default_config() -> SelfFieldConfig {
        SelfFieldConfig {
            clock: Some(Arc::new(aletheon_kernel::chronos::TestClock::default())),
            ..SelfFieldConfig::default()
        }
    }

    fn make_intent(action: &str, description: &str) -> Intent {
        Intent {
            action: action.to_string(),
            parameters: json!({}),
            source: IntentSource::User,
            description: description.to_string(),
        }
    }

    fn minimal_ctx() -> Context {
        Context::new("test", PathBuf::from("/tmp"))
    }

    #[tokio::test]
    async fn review_allow() {
        let sf = SelfField::new(default_config());
        let intent = make_intent("ls", "list files");
        let ctx = minimal_ctx();
        let verdict = sf.review(&intent, &ctx).await.unwrap();
        assert!(matches!(verdict, Verdict::Allow));
    }

    #[tokio::test]
    async fn review_deny_by_policy_bridge() {
        // "rm -rf /" matches the PolicyEngine default rule "rm -rf *" -> RequireApproval
        // which maps to Verdict::RequireConfirmation. The policy bridge runs before boundary.
        let sf = SelfField::new(default_config());
        let intent = make_intent("rm -rf /", "delete everything");
        let ctx = minimal_ctx();
        let verdict = sf.review(&intent, &ctx).await.unwrap();
        assert!(matches!(verdict, Verdict::RequireConfirmation { .. }));
    }

    #[tokio::test]
    async fn review_deny_by_boundary() {
        // Use an action that is NOT caught by the policy bridge default rules
        // but IS caught by a custom boundary rule
        let mut config = default_config();
        config.boundary_rules.push(BoundaryRule {
            action_pattern: "purge *".to_string(),
            source_filter: None,
            action: crate::core::boundary::BoundaryAction::Deny,
            risk_level: AwarenessRiskLevel::Critical,
            description: "no purge".to_string(),
            immutable: false,
        });
        let sf = SelfField::new(config);
        let intent = make_intent("purge data", "purge all data");
        let ctx = minimal_ctx();
        let verdict = sf.review(&intent, &ctx).await.unwrap();
        assert!(matches!(verdict, Verdict::Deny { .. }));
    }

    #[tokio::test]
    async fn review_sandbox_by_boundary() {
        let mut config = default_config();
        config.boundary_rules.push(BoundaryRule {
            action_pattern: "deploy.*".to_string(),
            source_filter: None,
            action: crate::core::boundary::BoundaryAction::Sandbox,
            risk_level: AwarenessRiskLevel::High,
            description: "sandbox deploys".to_string(),
            immutable: false,
        });
        let sf = SelfField::new(config);
        let intent = make_intent("deploy.prod", "deploy to production");
        let ctx = minimal_ctx();
        let verdict = sf.review(&intent, &ctx).await.unwrap();
        assert!(matches!(verdict, Verdict::SandboxFirst { .. }));
    }

    #[tokio::test]
    async fn review_confirm_by_boundary() {
        let mut config = default_config();
        config.boundary_rules.push(BoundaryRule {
            action_pattern: "write.*".to_string(),
            source_filter: None,
            action: crate::core::boundary::BoundaryAction::RequireConfirmation,
            risk_level: AwarenessRiskLevel::Medium,
            description: "confirm writes".to_string(),
            immutable: false,
        });
        let sf = SelfField::new(config);
        let intent = make_intent("write.config", "write config file");
        let ctx = minimal_ctx();
        let verdict = sf.review(&intent, &ctx).await.unwrap();
        assert!(matches!(verdict, Verdict::RequireConfirmation { .. }));
    }

    #[tokio::test]
    async fn identity_returns_current() {
        let sf = SelfField::new(default_config());
        let id = sf.identity().await.unwrap();
        assert_eq!(id.name, "aletheon");
    }

    #[tokio::test]
    async fn cares_returns_default() {
        let sf = SelfField::new(default_config());
        let cares = sf.cares().await.unwrap();
        assert_eq!(cares.len(), 4);
    }

    #[tokio::test]
    async fn narrate_records() {
        let sf = SelfField::new(default_config());
        sf.narrate("test_event", "test reason").await.unwrap();
        let entries = sf.narrative.recent(10);
        assert!(entries.iter().any(|e| e.event == "test_event"));
    }

    #[tokio::test]
    async fn resolve_conflict_works() {
        let sf = SelfField::new(default_config());
        let conflict = Conflict {
            source_a: ConflictSource::User {
                intent: "do X".to_string(),
            },
            source_b: ConflictSource::Brain {
                proposal: "do Y".to_string(),
                confidence: 0.5,
            },
            context: minimal_ctx(),
        };
        let resolution = sf.resolve_conflict(&conflict).await.unwrap();
        assert!(matches!(resolution, Resolution::AcceptA { .. }));
    }

    #[tokio::test]
    async fn review_mutation_reversible() {
        let sf = SelfField::new(default_config());
        let mutation = MutationIntent {
            target: "care_priorities".to_string(),
            change: json!({"safety": 0.9}),
            reason: "adjusting".to_string(),
            reversible: true,
        };
        let verdict = sf.review_mutation(&mutation).await.unwrap();
        assert!(matches!(verdict, Verdict::Allow));
    }

    #[tokio::test]
    async fn subsystem_lifecycle() {
        let mut sf = SelfField::new(default_config());
        assert_eq!(sf.name(), "self_field");
        assert!(matches!(
            sf.health().await,
            SubsystemHealth::Degraded { .. }
        ));

        let ctx = SubsystemContext {
            name: "self_field".to_string(),
            working_dir: PathBuf::from("/tmp"),
            config: json!({}),
            bus: None,
        };
        sf.init(&ctx).await.unwrap();
        assert!(matches!(sf.health().await, SubsystemHealth::Healthy));

        sf.shutdown().await.unwrap();
        assert!(matches!(
            sf.health().await,
            SubsystemHealth::Degraded { .. }
        ));
    }

    use fabric::policy::permission_authority::PermissionAuthority;
    use std::sync::Arc;

    struct StubAuthority;
    impl PermissionAuthority for StubAuthority {
        fn confirmation_verdict(
            &self,
            _ctx: &fabric::Context,
            _care: f64,
            action: &str,
        ) -> Option<Verdict> {
            Some(Verdict::RequireConfirmation {
                reason: format!("stub gate for {action}"),
                risk_level: AwarenessRiskLevel::Medium,
            })
        }
    }

    #[tokio::test]
    async fn review_delegates_permission_verdict_to_authority() {
        let mut sf = SelfField::new(default_config());
        sf.set_permission_authority(Arc::new(StubAuthority));
        let intent = make_intent("settings.update", "update a setting");
        let ctx = minimal_ctx();
        let verdict = sf.review(&intent, &ctx).await.unwrap();
        assert!(
            matches!(verdict, Verdict::RequireConfirmation { .. }),
            "authority verdict must be honored, got {verdict:?}"
        );
    }

    #[tokio::test]
    async fn review_falls_back_to_inline_when_no_authority_installed() {
        let sf = SelfField::new(default_config());
        // Low-care action -- shouldn't trigger confirmation
        let intent = make_intent("ls", "list files");
        let ctx = minimal_ctx();
        let verdict = sf.review(&intent, &ctx).await.unwrap();
        assert!(
            matches!(verdict, Verdict::Allow),
            "no authority + low care = allow, got {verdict:?}"
        );
    }
}
