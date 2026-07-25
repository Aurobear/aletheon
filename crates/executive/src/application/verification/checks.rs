//! Deterministic non-command verification checks.

use super::command::severity;
use super::{ArchitecturePolicy, VerificationCheckKind, VerificationContext};
use fabric::{ChangedFileKind, VerificationCheck};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub fn diff_scope(context: &VerificationContext, fresh_status: &[u8]) -> VerificationCheck {
    let kind = VerificationCheckKind::DiffScope;
    let status_paths = match parse_porcelain_v2(fresh_status) {
        Ok(paths) => paths,
        Err(error) => return failed(kind, format!("invalid fresh git status: {error}"), vec![]),
    };
    let reported: BTreeSet<_> = context
        .changed_files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    let fresh: BTreeSet<_> = status_paths.into_iter().collect();
    let mut violations = Vec::new();
    for path in reported.union(&fresh) {
        if let Err(error) =
            validate_path_scope(path, &context.allowed_paths, &context.forbidden_paths)
        {
            violations.push(format!("{}: {error}", path.display()));
        }
    }
    if reported != fresh {
        let missing: Vec<_> = fresh
            .difference(&reported)
            .map(|path| path.display().to_string())
            .collect();
        let stale: Vec<_> = reported
            .difference(&fresh)
            .map(|path| path.display().to_string())
            .collect();
        if !missing.is_empty() {
            violations.push(format!(
                "fresh status has unreported paths: {}",
                missing.join(", ")
            ));
        }
        if !stale.is_empty() {
            violations.push(format!("report has stale paths: {}", stale.join(", ")));
        }
    }
    if violations.is_empty() {
        passed(
            kind,
            format!("{} changed paths are in scope", fresh.len()),
            vec![],
        )
    } else {
        failed(kind, "diff scope rejected".into(), violations)
    }
}

pub fn capability_policy(context: &VerificationContext) -> VerificationCheck {
    let kind = VerificationCheckKind::CapabilityPolicy;
    let audit = context.capability_audit.clone().normalized();
    if !audit.audit_present {
        return failed(kind, "capability audit is missing".into(), vec![]);
    }
    let allowed: BTreeSet<_> = audit.allowed_capabilities.iter().collect();
    let denied: Vec<_> = audit
        .observed_capabilities
        .iter()
        .filter(|capability| !allowed.contains(capability))
        .cloned()
        .collect();
    if denied.is_empty() {
        passed(
            kind,
            "capability audit matches allow-list".into(),
            audit
                .observed_capabilities
                .into_iter()
                .map(|item| format!("observed: {item}"))
                .collect(),
        )
    } else {
        failed(
            kind,
            "attempt used disallowed capabilities".into(),
            denied
                .into_iter()
                .map(|item| format!("disallowed: {item}"))
                .collect(),
        )
    }
}

pub fn architecture_review(
    context: &VerificationContext,
    policy: &ArchitecturePolicy,
) -> VerificationCheck {
    let kind = VerificationCheckKind::ArchitectureReview;
    let mut findings = Vec::new();
    for changed in &context.changed_files {
        let path = &changed.path;
        for forbidden in &policy.forbidden_path_prefixes {
            if path == forbidden || path.starts_with(forbidden) {
                findings.push(format!(
                    "forbidden architecture path changed: {}",
                    path.display()
                ));
            }
        }
        if changed.kind == ChangedFileKind::Deleted {
            continue;
        }
        let absolute = context.worktree.join(path);
        let safe_file = absolute
            .symlink_metadata()
            .ok()
            .filter(|metadata| !metadata.file_type().is_symlink())
            .and_then(|_| absolute.canonicalize().ok())
            .filter(|canonical| canonical.starts_with(&context.worktree));
        let Some(absolute) = safe_file else {
            findings.push(format!(
                "architecture input is not a safe worktree file: {}",
                path.display()
            ));
            continue;
        };
        if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            if let Ok(source) = std::fs::read_to_string(&absolute) {
                for prefix in &policy.forbidden_import_prefixes {
                    if source.lines().any(|line| {
                        let trimmed = line.trim_start();
                        trimmed.starts_with(&format!("use {prefix}"))
                            || trimmed.starts_with(&format!("pub use {prefix}"))
                    }) {
                        findings.push(format!("forbidden import {prefix} in {}", path.display()));
                    }
                }
            }
        }
        if path.file_name().and_then(|value| value.to_str()) == Some("Cargo.toml") {
            if let Ok(source) = std::fs::read_to_string(&absolute) {
                if let Ok(manifest) = source.parse::<toml::Value>() {
                    let package = manifest
                        .get("package")
                        .and_then(|value| value.get("name"))
                        .and_then(toml::Value::as_str);
                    if let Some(package) = package {
                        for edge in &policy.forbidden_dependency_edges {
                            if edge.from == package && has_dependency(&manifest, &edge.to) {
                                findings.push(format!(
                                    "forbidden dependency direction: {} -> {}",
                                    edge.from, edge.to
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    if findings.is_empty() {
        passed(
            kind,
            "no configured architecture rule violations".into(),
            vec![],
        )
    } else {
        failed(
            kind,
            "architecture review found policy risks".into(),
            findings,
        )
    }
}

pub fn failed(
    kind: VerificationCheckKind,
    summary: String,
    evidence: Vec<String>,
) -> VerificationCheck {
    VerificationCheck {
        name: kind.as_str().into(),
        severity: severity(kind),
        passed: false,
        timed_out: false,
        cancelled: false,
        summary,
        evidence,
    }
}

fn passed(
    kind: VerificationCheckKind,
    summary: String,
    evidence: Vec<String>,
) -> VerificationCheck {
    VerificationCheck {
        name: kind.as_str().into(),
        severity: severity(kind),
        passed: true,
        timed_out: false,
        cancelled: false,
        summary,
        evidence,
    }
}

fn validate_path_scope(
    path: &Path,
    allowed: &[PathBuf],
    forbidden: &[PathBuf],
) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("path is not repository-relative".into());
    }
    if forbidden
        .iter()
        .any(|entry| path == entry || path.starts_with(entry))
    {
        return Err("path is forbidden".into());
    }
    if !allowed
        .iter()
        .any(|entry| path == entry || path.starts_with(entry))
    {
        return Err("path is outside allowed scope".into());
    }
    Ok(())
}

fn parse_porcelain_v2(status: &[u8]) -> Result<Vec<PathBuf>, String> {
    let text = std::str::from_utf8(status).map_err(|_| "status contains non-UTF-8 path")?;
    let mut records = text.split('\0').filter(|record| !record.is_empty());
    let mut paths = Vec::new();
    while let Some(record) = records.next() {
        if let Some(path) = record.strip_prefix("? ") {
            paths.push(PathBuf::from(path));
        } else if record.starts_with("1 ") {
            let fields: Vec<&str> = record.splitn(9, ' ').collect();
            if fields.len() != 9 {
                return Err("malformed ordinary status record".into());
            }
            paths.push(PathBuf::from(fields[8]));
        } else if record.starts_with("2 ") {
            let fields: Vec<&str> = record.splitn(10, ' ').collect();
            if fields.len() != 10 {
                return Err("malformed rename status record".into());
            }
            paths.push(PathBuf::from(fields[9]));
            let _ = records.next();
        } else if record.starts_with("u ") {
            return Err("unmerged status record".into());
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn has_dependency(manifest: &toml::Value, dependency: &str) -> bool {
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .iter()
        .any(|section| {
            manifest
                .get(section)
                .and_then(toml::Value::as_table)
                .is_some_and(|table| table.contains_key(dependency))
        })
}
