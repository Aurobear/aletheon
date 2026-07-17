//! Executive-owned session extension policy and activation boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use corpus::{ActivationReceipt, ActivationRequest, CorpusService, ExtensionGrant};
use fabric::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity, EventPayload,
    EventSpine, EventTreeId, EventVisibility, ExtensionId, ExtensionSnapshot, NamespaceId,
    SchemaId, UnsequencedEvent,
};
use serde::{Deserialize, Serialize};

use crate::core::config::ConfigSource;

pub const EXTENSION_ACTIVATION_EVENT_V1: &str = "aletheon.event.extension_activation/v1";

#[derive(Debug, Clone, Default)]
pub struct SessionExtensionPolicy {
    /// Empty means all granted catalog entries are enabled by effective config.
    pub enabled: BTreeSet<ExtensionId>,
    pub agent_name: Option<String>,
    pub provenance: BTreeMap<String, ConfigSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionActivationDecision {
    pub schema: String,
    pub session_id: String,
    pub extension_id: String,
    pub approved: bool,
    pub reason: String,
    pub config_source: Option<ConfigSource>,
}

#[async_trait]
pub trait ExtensionDecisionSink: Send + Sync {
    async fn record(&self, decision: ExtensionActivationDecision) -> Result<()>;
}

pub struct SpineExtensionDecisionSink {
    spine: Arc<dyn EventSpine>,
}

impl SpineExtensionDecisionSink {
    pub fn new(spine: Arc<dyn EventSpine>) -> Self {
        Self { spine }
    }
}

#[async_trait]
impl ExtensionDecisionSink for SpineExtensionDecisionSink {
    async fn record(&self, decision: ExtensionActivationDecision) -> Result<()> {
        let payload = serde_json::to_value(&decision)?;
        let session_id = decision.session_id.clone();
        let envelope = EnvelopeV2::new(
            SchemaId(EXTENSION_ACTIVATION_EVENT_V1.into()),
            EnvelopeV2Target("executive:extension-policy".into()),
            EnvelopeV2Target(format!("session:{session_id}")),
            EnvelopeV2Delivery::FanOut,
            NamespaceId(format!("session:{session_id}")),
            payload.clone(),
        );
        self.spine.append(UnsequencedEvent {
            tree_id: EventTreeId::for_root_session(&session_id),
            event_id: EventId::new(),
            parent: None,
            identity: EventIdentity {
                root_session_id: session_id.clone(),
                session_id,
                agent_id: None,
            },
            envelope,
            visibility: EventVisibility::Control,
            payload: EventPayload::Inline { value: payload },
        })?;
        Ok(())
    }
}

#[derive(Default)]
pub struct NoopExtensionDecisionSink;

#[async_trait]
impl ExtensionDecisionSink for NoopExtensionDecisionSink {
    async fn record(&self, _decision: ExtensionActivationDecision) -> Result<()> {
        Ok(())
    }
}

pub struct ActivatedExtensions {
    pub snapshot: ExtensionSnapshot,
    pub receipt: ActivationReceipt,
}

/// The only Executive service that converts catalog discovery into session activation.
pub struct ExtensionService {
    corpus: Arc<dyn CorpusService>,
    decisions: Arc<dyn ExtensionDecisionSink>,
}

impl ExtensionService {
    pub fn new(corpus: Arc<dyn CorpusService>, decisions: Arc<dyn ExtensionDecisionSink>) -> Self {
        Self { corpus, decisions }
    }

    pub async fn activate(
        &self,
        grant: ExtensionGrant,
        requested: Vec<ExtensionId>,
        policy: &SessionExtensionPolicy,
    ) -> Result<ActivatedExtensions> {
        let snapshot = self.corpus.catalog(&grant).await?;
        let available: BTreeMap<_, _> = snapshot
            .entries
            .iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        let mut requested = requested;
        requested.sort();
        requested.dedup();
        let mut approved = Vec::new();
        for id in requested {
            let (allow, reason) = match available.get(&id) {
                None => (false, "not granted by capability policy".to_string()),
                Some(_) if !policy.enabled.is_empty() && !policy.enabled.contains(&id) => {
                    (false, "disabled by effective configuration".to_string())
                }
                Some(descriptor)
                    if !descriptor.activation.allowed_agents.is_empty()
                        && policy.agent_name.as_ref().is_none_or(|agent| {
                            !descriptor.activation.allowed_agents.contains(agent)
                        }) =>
                {
                    (false, "not allowed for this agent".to_string())
                }
                Some(_) => (true, "approved by scoped capability grant".to_string()),
            };
            let source = policy.provenance.get(id.as_str()).cloned();
            self.decisions
                .record(ExtensionActivationDecision {
                    schema: EXTENSION_ACTIVATION_EVENT_V1.into(),
                    session_id: grant.session_id.clone(),
                    extension_id: id.as_str().to_string(),
                    approved: allow,
                    reason,
                    config_source: source,
                })
                .await?;
            if allow {
                approved.push(id);
            }
        }
        let receipt = self
            .corpus
            .activate(ActivationRequest {
                grant,
                extensions: approved,
            })
            .await?;
        Ok(ActivatedExtensions { snapshot, receipt })
    }
}
