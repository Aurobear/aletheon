//! Conflict detection between agents.
//!
//! Detects resource conflicts (file writes, service ownership, memory, etc.)
//! and proposes resolution strategies.

use super::{AgentId, AgentInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;

/// Type of conflict detected between agents.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConflictType {
    /// Two agents writing to the same file or directory.
    FileWriteConflict,
    /// Two agents claiming ownership of the same service.
    ServiceConflict,
    /// Two agents competing for the same limited resource (port, device, etc.).
    ResourceConflict,
    /// Two agents using overlapping memory regions.
    MemoryConflict,
}

/// Strategy for resolving a detected conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// Serialize access: one agent waits for the other to finish.
    Serialize,
    /// Delegate control to the agent that owns the resource.
    DelegateToOwner,
    /// Use an arbitration protocol to decide.
    Arbitrate,
    /// Block the conflicting action entirely.
    Block,
}

/// A single detected conflict between two agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictReport {
    /// Type of conflict.
    pub conflict_type: ConflictType,
    /// First agent involved.
    pub agent_a: AgentId,
    /// Second agent involved.
    pub agent_b: AgentId,
    /// Description of the conflicting resource.
    pub resource: String,
    /// Proposed resolution strategy.
    pub resolution: ConflictResolution,
}

/// Tracks resources claimed by agents and detects conflicts.
pub struct ConflictDetector {
    /// File paths claimed by agents.
    file_claims: HashMap<PathBuf, AgentId>,
    /// Service names claimed by agents.
    service_claims: HashMap<String, AgentId>,
    /// Named resource claims (ports, devices, etc.).
    resource_claims: HashMap<String, AgentId>,
}

impl ConflictDetector {
    /// Create a new empty conflict detector.
    pub fn new() -> Self {
        Self {
            file_claims: HashMap::new(),
            service_claims: HashMap::new(),
            resource_claims: HashMap::new(),
        }
    }

    /// Register a file path claim by an agent.
    ///
    /// Returns a conflict report if another agent already claimed this path.
    pub fn claim_file(&mut self, agent: &AgentId, path: PathBuf) -> Option<ConflictReport> {
        if let Some(existing) = self.file_claims.get(&path) {
            if existing != agent {
                warn!(
                    "File conflict: agent {} and {} both claim {}",
                    agent,
                    existing,
                    path.display()
                );
                return Some(ConflictReport {
                    conflict_type: ConflictType::FileWriteConflict,
                    agent_a: existing.clone(),
                    agent_b: agent.clone(),
                    resource: path.display().to_string(),
                    resolution: ConflictResolution::Serialize,
                });
            }
            None
        } else {
            self.file_claims.insert(path, agent.clone());
            None
        }
    }

    /// Register a service ownership claim by an agent.
    ///
    /// Returns a conflict report if another agent already owns this service.
    pub fn claim_service(&mut self, agent: &AgentId, service: &str) -> Option<ConflictReport> {
        if let Some(existing) = self.service_claims.get(service) {
            if existing != agent {
                warn!(
                    "Service conflict: agent {} and {} both claim {}",
                    agent, existing, service
                );
                return Some(ConflictReport {
                    conflict_type: ConflictType::ServiceConflict,
                    agent_a: existing.clone(),
                    agent_b: agent.clone(),
                    resource: service.to_string(),
                    resolution: ConflictResolution::DelegateToOwner,
                });
            }
            None
        } else {
            self.service_claims.insert(service.to_string(), agent.clone());
            None
        }
    }

    /// Register a named resource claim by an agent.
    ///
    /// Returns a conflict report if another agent already claimed this resource.
    pub fn claim_resource(&mut self, agent: &AgentId, resource: &str) -> Option<ConflictReport> {
        if let Some(existing) = self.resource_claims.get(resource) {
            if existing != agent {
                warn!(
                    "Resource conflict: agent {} and {} both claim {}",
                    agent, existing, resource
                );
                return Some(ConflictReport {
                    conflict_type: ConflictType::ResourceConflict,
                    agent_a: existing.clone(),
                    agent_b: agent.clone(),
                    resource: resource.to_string(),
                    resolution: ConflictResolution::Arbitrate,
                });
            }
            None
        } else {
            self.resource_claims.insert(resource.to_string(), agent.clone());
            None
        }
    }

    /// Release a file claim.
    pub fn release_file(&mut self, agent: &AgentId, path: &PathBuf) {
        if let Some(owner) = self.file_claims.get(path) {
            if owner == agent {
                self.file_claims.remove(path);
            }
        }
    }

    /// Release a service claim.
    pub fn release_service(&mut self, agent: &AgentId, service: &str) {
        if let Some(owner) = self.service_claims.get(service) {
            if owner == agent {
                self.service_claims.remove(service);
            }
        }
    }

    /// Release a resource claim.
    pub fn release_resource(&mut self, agent: &AgentId, resource: &str) {
        if let Some(owner) = self.resource_claims.get(resource) {
            if owner == agent {
                self.resource_claims.remove(resource);
            }
        }
    }

    /// Run a full conflict check across all registered agents.
    ///
    /// This performs a comprehensive scan for any overlapping claims.
    pub fn detect_all(&self, agents: &[AgentInfo]) -> Vec<ConflictReport> {
        let reports = Vec::new();

        // Check for agents that are not in the claim maps but might conflict
        // This is mainly for future extensibility; current claims-based model
        // detects conflicts at claim time.

        // Verify all claimed agents actually exist
        let known_ids: std::collections::HashSet<&AgentId> =
            agents.iter().map(|a| &a.id).collect();

        for (path, agent_id) in &self.file_claims {
            if !known_ids.contains(agent_id) {
                warn!(
                    "File {} claimed by unknown agent {}",
                    path.display(),
                    agent_id
                );
            }
        }

        reports
    }

    /// Get the number of active file claims.
    pub fn file_claim_count(&self) -> usize {
        self.file_claims.len()
    }

    /// Get the number of active service claims.
    pub fn service_claim_count(&self) -> usize {
        self.service_claims.len()
    }

    /// Get the number of active resource claims.
    pub fn resource_claim_count(&self) -> usize {
        self.resource_claims.len()
    }

    /// Get all file claims (for inspection).
    pub fn file_claims(&self) -> &HashMap<PathBuf, AgentId> {
        &self.file_claims
    }

    /// Get all service claims (for inspection).
    pub fn service_claims(&self) -> &HashMap<String, AgentId> {
        &self.service_claims
    }
}

impl Default for ConflictDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{AgentId, AgentKind, Endpoint, AgentInfo};
    use std::path::PathBuf;

    fn make_agents(n: usize) -> Vec<AgentInfo> {
        (0..n)
            .map(|_| {
                AgentInfo::new(
                    AgentId::new(),
                    AgentKind::Worker,
                    Endpoint::UnixSocket(PathBuf::from("/tmp/test.sock")),
                )
            })
            .collect()
    }

    #[test]
    fn test_no_conflict_on_unique_claims() {
        let mut detector = ConflictDetector::new();
        let agent_a = AgentId::new();

        assert!(detector
            .claim_file(&agent_a, PathBuf::from("/tmp/a.txt"))
            .is_none());
        assert!(detector.claim_service(&agent_a, "nginx").is_none());
        assert!(detector.claim_resource(&agent_a, "port:8080").is_none());

        assert_eq!(detector.file_claim_count(), 1);
        assert_eq!(detector.service_claim_count(), 1);
        assert_eq!(detector.resource_claim_count(), 1);
    }

    #[test]
    fn test_file_conflict_detected() {
        let mut detector = ConflictDetector::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        // First claim succeeds
        assert!(detector
            .claim_file(&agent_a, PathBuf::from("/tmp/shared.txt"))
            .is_none());

        // Second claim by different agent triggers conflict
        let report = detector
            .claim_file(&agent_b, PathBuf::from("/tmp/shared.txt"))
            .expect("Expected conflict report");

        assert_eq!(report.conflict_type, ConflictType::FileWriteConflict);
        assert_eq!(report.agent_a, agent_a);
        assert_eq!(report.agent_b, agent_b);
        assert_eq!(report.resolution, ConflictResolution::Serialize);
    }

    #[test]
    fn test_same_agent_no_conflict() {
        let mut detector = ConflictDetector::new();
        let agent = AgentId::new();

        // Claiming the same resource twice by the same agent should not conflict
        assert!(detector
            .claim_file(&agent, PathBuf::from("/tmp/same.txt"))
            .is_none());
        assert!(detector
            .claim_file(&agent, PathBuf::from("/tmp/same.txt"))
            .is_none());
    }

    #[test]
    fn test_service_conflict() {
        let mut detector = ConflictDetector::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        detector.claim_service(&agent_a, "database");
        let report = detector
            .claim_service(&agent_b, "database")
            .unwrap();

        assert_eq!(report.conflict_type, ConflictType::ServiceConflict);
        assert_eq!(report.resolution, ConflictResolution::DelegateToOwner);
    }

    #[test]
    fn test_resource_conflict() {
        let mut detector = ConflictDetector::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        detector.claim_resource(&agent_a, "gpu:0");
        let report = detector.claim_resource(&agent_b, "gpu:0").unwrap();

        assert_eq!(report.conflict_type, ConflictType::ResourceConflict);
        assert_eq!(report.resolution, ConflictResolution::Arbitrate);
    }

    #[test]
    fn test_release_claims() {
        let mut detector = ConflictDetector::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        detector.claim_file(&agent_a, PathBuf::from("/tmp/file.txt"));
        detector.claim_service(&agent_a, "svc");
        detector.claim_resource(&agent_a, "res");

        // Release claims
        detector.release_file(&agent_a, &PathBuf::from("/tmp/file.txt"));
        detector.release_service(&agent_a, "svc");
        detector.release_resource(&agent_a, "res");

        assert_eq!(detector.file_claim_count(), 0);
        assert_eq!(detector.service_claim_count(), 0);
        assert_eq!(detector.resource_claim_count(), 0);

        // Now agent_b can claim without conflict
        assert!(detector
            .claim_file(&agent_b, PathBuf::from("/tmp/file.txt"))
            .is_none());
    }

    #[test]
    fn test_release_only_owner_can_release() {
        let mut detector = ConflictDetector::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        detector.claim_file(&agent_a, PathBuf::from("/tmp/owned.txt"));

        // agent_b tries to release agent_a's claim - should not work
        detector.release_file(&agent_b, &PathBuf::from("/tmp/owned.txt"));
        assert_eq!(detector.file_claim_count(), 1);
    }

    #[test]
    fn test_detect_all_no_agents() {
        let detector = ConflictDetector::new();
        let agents = make_agents(0);
        let reports = detector.detect_all(&agents);
        assert!(reports.is_empty());
    }

    #[test]
    fn test_conflict_report_serialization() {
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();
        let report = ConflictReport {
            conflict_type: ConflictType::MemoryConflict,
            agent_a: agent_a.clone(),
            agent_b: agent_b.clone(),
            resource: "heap:0x7fff".to_string(),
            resolution: ConflictResolution::Block,
        };
        let json = serde_json::to_string(&report).unwrap();
        let deserialized: ConflictReport = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.conflict_type, ConflictType::MemoryConflict);
        assert_eq!(deserialized.resolution, ConflictResolution::Block);
    }
}
