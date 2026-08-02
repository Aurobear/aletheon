//! Fail-closed launch-profile resolution for native cognitive roles.

use std::collections::{HashMap, HashSet};

use fabric::cognitive_workflow::CognitiveRole;
use fabric::AgentProfile;

use crate::application::cognitive_role_workflow::RoleLaunchProfile;

pub(super) const ROLE_PROFILE_IDS: [(CognitiveRole, &str); 6] = [
    (CognitiveRole::Planner, "planner-agent"),
    (CognitiveRole::Explorer, "explorer-agent"),
    (CognitiveRole::Executor, "executor-agent"),
    (CognitiveRole::Tester, "tester-agent"),
    (CognitiveRole::Reviewer, "reviewer-agent"),
    (CognitiveRole::Fixer, "fixer-agent"),
];

const READ_TOOLS: &[&str] = &[
    "repo_inspect",
    "file_read",
    "artifact_read",
    "grep",
    "glob",
    "file_search",
    "code_graph",
];

const WRITE_TOOLS: &[&str] = &[
    "file_write",
    "apply_patch",
    "exec_command",
    "write_stdin",
    "validation_run",
    "change_accept",
    "change_rollback",
];

fn explicit_tools(role: CognitiveRole) -> anyhow::Result<Vec<&'static str>> {
    let mut tools = READ_TOOLS.to_vec();
    match role {
        CognitiveRole::Planner | CognitiveRole::Explorer | CognitiveRole::Reviewer => {}
        CognitiveRole::Tester => tools.push("validation_run"),
        CognitiveRole::Executor | CognitiveRole::Fixer => tools.extend_from_slice(WRITE_TOOLS),
        CognitiveRole::Root => anyhow::bail!("root is not a launchable native cognitive role"),
    }
    Ok(tools)
}

fn permitted_tools(role: CognitiveRole) -> anyhow::Result<HashSet<&'static str>> {
    let mut permitted = explicit_tools(role)?.into_iter().collect::<HashSet<_>>();
    permitted.extend(super::runtime::UNIVERSAL_TOOLS.iter().copied());
    Ok(permitted)
}

pub(super) fn resolve_role_launch_profiles(
    profiles: &HashMap<String, AgentProfile>,
) -> anyhow::Result<HashMap<CognitiveRole, RoleLaunchProfile>> {
    let mut launches = HashMap::new();
    for (role, expected_id) in ROLE_PROFILE_IDS {
        let profile = profiles
            .get(expected_id)
            .ok_or_else(|| anyhow::anyhow!("missing required cognitive profile '{expected_id}'"))?;
        anyhow::ensure!(
            profile.id.0 == expected_id,
            "cognitive profile identity mismatch for {role:?}: expected '{expected_id}', got '{}'",
            profile.id.0
        );

        let actual = profile
            .allowed_tools
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let expected = permitted_tools(role)?;
        anyhow::ensure!(
            actual.len() == profile.allowed_tools.len(),
            "cognitive profile '{expected_id}' contains duplicate effective tools"
        );
        anyhow::ensure!(
            actual == expected,
            "cognitive profile '{expected_id}' effective tool contract mismatch: expected {expected:?}, got {actual:?}"
        );
        launches.insert(
            role,
            RoleLaunchProfile {
                profile_id: profile.id.clone(),
                allowed_tools: profile.allowed_tools.clone(),
            },
        );
    }
    anyhow::ensure!(
        launches.len() == ROLE_PROFILE_IDS.len(),
        "native cognitive role launch profiles are not distinct"
    );
    Ok(launches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(role: CognitiveRole, id: &str) -> AgentProfile {
        let mut allowed_tools = permitted_tools(role)
            .unwrap()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        allowed_tools.sort();
        AgentProfile {
            id: fabric::AgentProfileId(id.into()),
            system_prompt: format!("{role:?} profile"),
            model: "test-model".into(),
            allowed_tools,
            max_iterations: 20,
            max_input_tokens: 8_000,
            max_output_tokens: 1_000,
            max_tool_calls: 16,
            max_elapsed_ms: 10_000,
            profile_name: id.into(),
            risk_tier: fabric::RiskTier::ReadOnly,
            approval_policy: fabric::AgentApprovalPolicy::AutoApprove,
            tool_timeout_ms: 5_000,
            inheritable: true,
            parent_restriction: fabric::ParentRestriction::SameOrSafer,
        }
    }

    fn exact_test_profiles() -> HashMap<String, AgentProfile> {
        ROLE_PROFILE_IDS
            .into_iter()
            .map(|(role, id)| (id.to_owned(), profile(role, id)))
            .collect()
    }

    #[test]
    fn every_cognitive_role_has_a_distinct_profile_id() {
        let ids = ROLE_PROFILE_IDS.map(|(_, id)| id);
        let unique = ids.into_iter().collect::<HashSet<_>>();
        assert_eq!(unique.len(), 6);

        let launches = resolve_role_launch_profiles(&exact_test_profiles()).unwrap();
        let launch_ids = launches
            .values()
            .map(|launch| launch.profile_id.0.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(launch_ids.len(), 6);
    }

    #[test]
    fn read_only_roles_reject_mutation_tools() {
        for role in [
            CognitiveRole::Planner,
            CognitiveRole::Explorer,
            CognitiveRole::Tester,
            CognitiveRole::Reviewer,
        ] {
            let permitted = permitted_tools(role).unwrap();
            for forbidden in [
                "file_write",
                "apply_patch",
                "exec_command",
                "write_stdin",
                "change_accept",
                "change_rollback",
            ] {
                assert!(!permitted.contains(forbidden), "{role:?} admitted {forbidden}");
            }
        }
        assert!(permitted_tools(CognitiveRole::Tester)
            .unwrap()
            .contains("validation_run"));
    }

    #[test]
    fn resolver_rejects_missing_renamed_underpowered_or_overpowered_profile() {
        let mut profiles = exact_test_profiles();
        profiles.remove("reviewer-agent");
        assert!(resolve_role_launch_profiles(&profiles)
            .unwrap_err()
            .to_string()
            .contains("reviewer-agent"));

        let mut profiles = exact_test_profiles();
        profiles.get_mut("planner-agent").unwrap().id.0 = "renamed".into();
        assert!(resolve_role_launch_profiles(&profiles)
            .unwrap_err()
            .to_string()
            .contains("identity mismatch"));

        let mut profiles = exact_test_profiles();
        profiles
            .get_mut("tester-agent")
            .unwrap()
            .allowed_tools
            .retain(|tool| tool != "validation_run");
        assert!(resolve_role_launch_profiles(&profiles)
            .unwrap_err()
            .to_string()
            .contains("tool contract mismatch"));

        let mut profiles = exact_test_profiles();
        profiles
            .get_mut("planner-agent")
            .unwrap()
            .allowed_tools
            .push("file_write".into());
        assert!(resolve_role_launch_profiles(&profiles)
            .unwrap_err()
            .to_string()
            .contains("tool contract mismatch"));
    }
}
