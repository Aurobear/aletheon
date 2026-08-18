//! Read-only extension package inspection owned by the CLI composition root.
//!
//! Installation, activation, and rollback are daemon application concerns;
//! this seam intentionally exposes only archive inspection so an offline CLI
//! command does not initialize the daemon's package store.

use std::path::Path;

/// Inspect an extension archive without creating or mutating package state.
pub fn inspect_archive(
    package_path: &Path,
) -> anyhow::Result<corpus::extension::inspector::InspectionResult> {
    corpus::extension::inspector::inspect_package(package_path)
}
