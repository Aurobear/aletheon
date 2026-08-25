//! K2 sealed descriptor + `CapabilityExecutor` (Agent Kernel V2).
//!
//! A sealed capability descriptor binds a stable capability id to an exact
//! executor implementation via a version + invocation digest.  The composition
//! root builds the registry, validates duplicate/version/digest, and **seals**
//! it — after sealing the descriptor-to-executor binding cannot be replaced
//! (no Runtime/Executive can inject an arbitrary executor).  This module is
//! contract/port only and is not part of the production invocation path.
//! `DefaultCapabilityInvoker` is the explicit production authority.

use ::contracts::CapabilityId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Invocation digest: a content hash that pins the exact executable semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationDigest(pub String);

/// A sealed capability descriptor.  Immutable after `CapabilityRegistry::seal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedDescriptor {
    pub capability: CapabilityId,
    pub version: u64,
    pub digest: InvocationDigest,
}

/// A capability executor bound to a sealed descriptor.  Executors must not
/// perform side effects before receiving a validated permit. This experimental
/// registry does not replace the production invoker.
#[async_trait::async_trait]
pub trait CapabilityExecutor: Send + Sync {
    /// Execute the capability for the given sealed descriptor and input.
    async fn execute(
        &self,
        descriptor: &SealedDescriptor,
        input: serde_json::Value,
    ) -> serde_json::Value;
}

/// No-op representative executor (K2: "先接一个无外部副作用的 representative
/// executor").  Used to verify the registry/seal path without any side effect.
#[derive(Debug, Default)]
pub struct NoopExecutor;

#[async_trait::async_trait]
impl CapabilityExecutor for NoopExecutor {
    async fn execute(
        &self,
        descriptor: &SealedDescriptor,
        input: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "capability": descriptor.capability.0,
            "version": descriptor.version,
            "echo": input,
        })
    }
}

/// Registry that validates and seals descriptor → executor bindings.
#[derive(Default)]
pub struct CapabilityRegistry {
    pending:
        std::collections::HashMap<CapabilityId, (SealedDescriptor, Arc<dyn CapabilityExecutor>)>,
    sealed: Option<Arc<std::collections::HashMap<CapabilityId, Arc<dyn CapabilityExecutor>>>>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a descriptor + executor.  Rejects duplicate capability ids,
    /// empty digests, or version 0.
    pub fn register(
        &mut self,
        descriptor: SealedDescriptor,
        executor: Arc<dyn CapabilityExecutor>,
    ) -> Result<(), String> {
        if self.sealed.is_some() {
            return Err("registry is sealed; binding cannot be replaced".into());
        }
        if descriptor.version == 0 {
            return Err("descriptor version must be >= 1".into());
        }
        if descriptor.digest.0.trim().is_empty() {
            return Err("descriptor digest must not be empty".into());
        }
        if self.pending.contains_key(&descriptor.capability) {
            return Err(format!("duplicate capability: {}", descriptor.capability.0));
        }
        self.pending
            .insert(descriptor.capability.clone(), (descriptor, executor));
        Ok(())
    }

    /// Seal the registry.  After this, bindings are immutable.
    pub fn seal(&mut self) -> Result<(), String> {
        if self.sealed.is_some() {
            return Err("registry already sealed".into());
        }
        let mut map = std::collections::HashMap::new();
        for (id, (_desc, executor)) in self.pending.drain() {
            map.insert(id, executor);
        }
        self.sealed = Some(Arc::new(map));
        Ok(())
    }

    /// Resolve an executor for a sealed capability.  Returns Err if the
    /// registry is not yet sealed or the capability is unknown.
    pub fn resolve(
        &self,
        capability: &CapabilityId,
    ) -> Result<Arc<dyn CapabilityExecutor>, String> {
        let sealed = self
            .sealed
            .as_ref()
            .ok_or_else(|| "registry not sealed".to_string())?;
        sealed
            .get(capability)
            .cloned()
            .ok_or_else(|| format!("unknown sealed capability: {}", capability.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(id: &str) -> SealedDescriptor {
        SealedDescriptor {
            capability: CapabilityId(id.into()),
            version: 1,
            digest: InvocationDigest("abc".into()),
        }
    }

    #[tokio::test]
    async fn register_seal_resolve_roundtrip() {
        let mut registry = CapabilityRegistry::new();
        registry
            .register(desc("tool.x"), Arc::new(NoopExecutor))
            .unwrap();
        registry.seal().unwrap();
        let executor = registry.resolve(&CapabilityId("tool.x".into())).unwrap();
        let out = executor
            .execute(&desc("tool.x"), serde_json::json!({"k": 1}))
            .await;
        assert_eq!(out["echo"]["k"], 1);
    }

    #[test]
    fn duplicate_capability_rejected() {
        let mut registry = CapabilityRegistry::new();
        registry
            .register(desc("tool.x"), Arc::new(NoopExecutor))
            .unwrap();
        assert!(registry
            .register(desc("tool.x"), Arc::new(NoopExecutor))
            .is_err());
    }

    #[test]
    fn seal_is_immutable() {
        let mut registry = CapabilityRegistry::new();
        registry
            .register(desc("tool.x"), Arc::new(NoopExecutor))
            .unwrap();
        registry.seal().unwrap();
        assert!(registry
            .register(desc("tool.y"), Arc::new(NoopExecutor))
            .is_err());
        assert!(registry.seal().is_err());
        assert!(registry.resolve(&CapabilityId("tool.y".into())).is_err());
    }
}
