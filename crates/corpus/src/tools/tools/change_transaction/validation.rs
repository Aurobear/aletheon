//! Repository-evidence-driven validation plan derivation.

use std::path::Path;

use super::super::repository::RepositoryContext;
use ::contracts::change_transaction::{
    ValidationImpact, ValidationPlanOmission, ValidationPlanStep, ValidationRisk,
};

pub(super) struct ValidationProjection {
    pub(super) steps: Vec<ValidationPlanStep>,
    pub(super) omissions: Vec<ValidationPlanOmission>,
    pub(super) impact: ValidationImpact,
    pub(super) risk: ValidationRisk,
}

impl Default for ValidationProjection {
    fn default() -> Self {
        Self {
            steps: Vec::new(),
            omissions: Vec::new(),
            impact: ValidationImpact::NonCode,
            risk: ValidationRisk::Low,
        }
    }
}

pub(super) fn derive_validation_plan(
    context: &RepositoryContext,
    changed_paths: &[String],
) -> ValidationProjection {
    let mut plan = Vec::new();
    let deployment_required = context.deployment_policy.as_ref().is_some_and(|policy| {
        policy.requires_installed_runtime
            && (policy.affected_path_prefixes.is_empty()
                || changed_paths.iter().any(|path| {
                    policy
                        .affected_path_prefixes
                        .iter()
                        .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
                }))
    });
    for spec in &context.validation_commands {
        if spec.kind == "deploy" && !deployment_required {
            continue;
        }
        if spec.command.is_empty()
            || spec.command.contains('<')
            || spec.command.contains('>')
            || spec.command.contains('$')
        {
            continue;
        }
        if plan.iter().any(|step: &ValidationPlanStep| {
            step.command == spec.command && step.validation_kind == spec.kind
        }) {
            continue;
        }
        plan.push(ValidationPlanStep {
            id: format!("repository-rule-{}-{}", spec.source_line, plan.len() + 1),
            validation_kind: spec.kind.clone(),
            command: spec.command.clone(),
            reason: "required by repository instruction evidence".into(),
            source: format!("{}:{}", spec.source_path, spec.source_line),
            required: true,
        });
    }
    if deployment_required {
        if let Some(policy) = &context.deployment_policy {
            if let Some(command) = policy
                .command
                .as_ref()
                .filter(|command| !command.is_empty())
            {
                if !plan.iter().any(|step| step.command == *command) {
                    plan.push(ValidationPlanStep {
                        id: "installed-runtime-acceptance".into(),
                        validation_kind: "deploy".into(),
                        command: command.clone(),
                        reason: "typed repository deployment policy applies to the changed paths"
                            .into(),
                        source: policy.source_path.clone(),
                        required: true,
                    });
                }
            }
        }
    }
    let rust_crate_dirs = changed_paths
        .iter()
        .filter_map(|path| {
            let mut parts = path.split('/');
            (parts.next() == Some("crates"))
                .then(|| parts.next())
                .flatten()
        })
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    let mut impact = if rust_crate_dirs.is_empty() {
        ValidationImpact::NonCode
    } else {
        ValidationImpact::PackageLocal
    };
    let manifest_changed = changed_paths
        .iter()
        .any(|path| path == "Cargo.toml" || path.ends_with("/Cargo.toml"));
    let mut dependency_checks = std::collections::BTreeSet::new();
    if context
        .manifests
        .iter()
        .any(|manifest| manifest.kind == "cargo")
    {
        let wrapper = Path::new(&context.root)
            .join("scripts/cargo-agent.sh")
            .is_file();
        let prefix = if wrapper {
            "bash scripts/cargo-agent.sh"
        } else {
            "cargo"
        };
        let packages = cargo_workspace_packages(Path::new(&context.root));
        for crate_dir in rust_crate_dirs {
            let crate_name = cargo_package_name(Path::new(&context.root), &crate_dir)
                .unwrap_or_else(|| crate_dir.clone());
            for (kind, suffix, reason) in [
                (
                    "check",
                    format!("check -p {crate_name}"),
                    format!("changed Rust package `{crate_name}` requires a target check"),
                ),
                (
                    "test",
                    format!("test -p {crate_name} --lib"),
                    format!("changed Rust package `{crate_name}` requires focused unit tests"),
                ),
            ] {
                let command = format!("{prefix} {suffix}");
                if !plan
                    .iter()
                    .any(|step| step.validation_kind == kind && step.command == command)
                {
                    plan.push(ValidationPlanStep {
                        id: format!("cargo-{kind}-{crate_name}"),
                        validation_kind: kind.into(),
                        command,
                        reason,
                        source: "manifest:Cargo.toml".into(),
                        required: true,
                    });
                }
            }
            let integration_prefix = format!("crates/{crate_dir}/tests/");
            let mut exact_integration_targets = std::collections::BTreeSet::new();
            let mut shared_integration_support_changed = false;
            for path in changed_paths
                .iter()
                .filter_map(|path| path.strip_prefix(&integration_prefix))
                .filter(|path| path.ends_with(".rs"))
            {
                if !path.contains('/')
                    && Path::new(&context.root)
                        .join(&integration_prefix)
                        .join(path)
                        .is_file()
                {
                    exact_integration_targets.insert(path.trim_end_matches(".rs").to_string());
                } else {
                    shared_integration_support_changed = true;
                }
            }
            for target in exact_integration_targets {
                let command = format!("{prefix} test -p {crate_name} --test {target}");
                plan.push(ValidationPlanStep {
                    id: format!("cargo-integration-{crate_name}-{target}"),
                    validation_kind: "test".into(),
                    command,
                    reason: format!(
                        "changed integration target `{target}` in Rust package `{crate_name}`"
                    ),
                    source: format!("manifest:crates/{crate_dir}/Cargo.toml"),
                    required: true,
                });
            }
            if shared_integration_support_changed {
                plan.push(ValidationPlanStep {
                    id: format!("cargo-integration-{crate_name}-shared"),
                    validation_kind: "test".into(),
                    command: format!("{prefix} test -p {crate_name} --tests"),
                    reason: format!(
                        "shared integration-test support changed in Rust package `{crate_name}`"
                    ),
                    source: format!("manifest:crates/{crate_dir}/Cargo.toml"),
                    required: true,
                });
            }
            for (dependent, dependencies) in &packages {
                if dependent != &crate_name && dependencies.contains(&crate_name) {
                    dependency_checks.insert(dependent.clone());
                }
            }
        }
        for dependent in dependency_checks {
            impact = ValidationImpact::WorkspaceDependency;
            plan.push(ValidationPlanStep {
                id: format!("cargo-dependent-check-{dependent}"),
                validation_kind: "check".into(),
                command: format!("{prefix} check -p {dependent}"),
                reason: format!(
                    "direct workspace dependent `{dependent}` must compile against the changed package"
                ),
                source: "workspace dependency graph".into(),
                required: true,
            });
        }
    }
    plan.sort_by_key(|step| match step.validation_kind.as_str() {
        "format" => 0,
        "check" => 1,
        "test" => 2,
        "lint" => 3,
        "build" => 4,
        "deploy" => 5,
        _ => 6,
    });
    let mut omissions = Vec::new();
    if !plan.iter().any(|step| step.validation_kind == "format") {
        omissions.push(ValidationPlanOmission {
            validation_kind: "format".into(),
            reason: "no applicable repository format command was found".into(),
        });
    }
    if !plan
        .iter()
        .any(|step| step.id.starts_with("cargo-integration-"))
    {
        omissions.push(ValidationPlanOmission {
            validation_kind: "integration_test".into(),
            reason: "no integration-test target or shared integration support changed".into(),
        });
    }
    if !deployment_required {
        omissions.push(ValidationPlanOmission {
            validation_kind: "installed_runtime".into(),
            reason: "typed repository metadata does not require installed-runtime acceptance"
                .into(),
        });
    }
    if !plan.iter().any(|step| step.validation_kind == "build") {
        omissions.push(ValidationPlanOmission {
            validation_kind: "workspace_build".into(),
            reason: "targeted package checks cover the derived dependency impact; no workspace-wide build was selected".into(),
        });
    }
    let risk = if deployment_required {
        ValidationRisk::DeploymentCritical
    } else if manifest_changed || impact == ValidationImpact::WorkspaceDependency {
        ValidationRisk::High
    } else if impact == ValidationImpact::PackageLocal {
        ValidationRisk::Moderate
    } else {
        ValidationRisk::Low
    };
    ValidationProjection {
        steps: plan,
        omissions,
        impact,
        risk,
    }
}

fn cargo_package_name(root: &Path, crate_dir: &str) -> Option<String> {
    let content =
        std::fs::read_to_string(root.join("crates").join(crate_dir).join("Cargo.toml")).ok()?;
    content
        .parse::<toml::Value>()
        .ok()?
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

fn cargo_workspace_packages(root: &Path) -> Vec<(String, std::collections::BTreeSet<String>)> {
    let Ok(entries) = std::fs::read_dir(root.join("crates")) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let manifest = std::fs::read_to_string(entry.path().join("Cargo.toml")).ok()?;
            let value = manifest.parse::<toml::Value>().ok()?;
            let package = value.get("package")?.get("name")?.as_str()?.to_string();
            let mut dependencies = std::collections::BTreeSet::new();
            for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(table) = value.get(table_name).and_then(toml::Value::as_table) {
                    dependencies.extend(table.keys().cloned());
                    dependencies.extend(table.values().filter_map(|dependency| {
                        dependency
                            .as_table()
                            .and_then(|table| table.get("package"))
                            .and_then(toml::Value::as_str)
                            .map(str::to_string)
                    }));
                }
            }
            Some((package, dependencies))
        })
        .collect()
}
