use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileKind { Core, Coding, Personal, Conscious, Evolution, HardwareEdge }

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FeatureSet {
    pub memory: bool,
    pub agora: bool,
    pub dasein: bool,
    pub metacog: bool,
    pub pi_runtime: bool,
    pub verifier: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentProfile {
    pub kind: ProfileKind,
    pub features: FeatureSet,
    pub required_ports: Vec<String>,
    pub optional_ports: Vec<String>,
    pub storage_paths: Vec<String>,
}

impl DeploymentProfile {
    pub fn core() -> Self {
        Self {
            kind: ProfileKind::Core,
            features: Default::default(),
            required_ports: vec!["inference".into(), "capabilities".into(), "sessions".into()],
            optional_ports: vec![],
            storage_paths: vec!["events.db".into()],
        }
    }

    pub fn coding() -> Self {
        Self {
            kind: ProfileKind::Coding,
            features: FeatureSet { pi_runtime: true, verifier: true, ..Default::default() },
            required_ports: vec!["inference".into(), "capabilities".into(), "sessions".into()],
            optional_ports: vec!["memory".into()],
            storage_paths: vec!["events.db".into(), "worktrees/".into()],
        }
    }

    pub fn is_default_production(&self) -> bool {
        matches!(self.kind, ProfileKind::Coding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_profile_enables_pi_and_verifier() {
        let p = DeploymentProfile::coding();
        assert!(p.features.pi_runtime);
        assert!(p.features.verifier);
    }

    #[test]
    fn core_profile_has_no_optional_features() {
        let p = DeploymentProfile::core();
        assert!(!p.features.memory);
        assert!(!p.features.metacog);
        assert!(p.optional_ports.is_empty());
    }
}
