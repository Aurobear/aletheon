//! Deterministic resolution of enabled package assets into contained paths.

use std::path::{Path, PathBuf};

use crate::extension::asset::AssetKind;
use crate::extension::package::AssetRef;
use anyhow::{Context, Result};

use super::store::{ActivationRecord, InstalledPackageRecord, PackageStore};

#[derive(Debug, Clone)]
pub struct ResolvedPackageAsset {
    pub package_id: String,
    pub package_version: String,
    pub package_hash: String,
    pub asset: AssetRef,
    pub absolute_path: PathBuf,
}

impl ResolvedPackageAsset {
    pub fn path(&self) -> &Path {
        &self.absolute_path
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedPackageSet {
    pub assets: Vec<ResolvedPackageAsset>,
    pub activation_records: Vec<ActivationRecord>,
}

impl ResolvedPackageSet {
    pub fn asset_ids(&self) -> Vec<&str> {
        self.assets
            .iter()
            .map(|resolved| resolved.asset.id.as_str())
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ResolvedPackageAsset> {
        self.assets.iter()
    }
}

pub struct PackageAssetResolver {
    store: PackageStore,
}

impl PackageAssetResolver {
    pub fn new(store: PackageStore) -> Self {
        Self { store }
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self::new(PackageStore::new(root.into())?))
    }

    pub fn resolve_enabled(&self) -> Result<ResolvedPackageSet> {
        let mut assets = Vec::new();
        let mut activation_records = Vec::new();
        for activation in self.store.list_activations()? {
            if !activation.enabled {
                continue;
            }
            let resolved = self.resolve_activation(&activation)?;
            assets.extend(resolved.assets);
            activation_records.extend(resolved.activation_records);
        }

        assets.sort_by(|left, right| {
            asset_kind_rank(left.asset.kind)
                .cmp(&asset_kind_rank(right.asset.kind))
                .then(left.asset.id.cmp(&right.asset.id))
                .then(left.package_id.cmp(&right.package_id))
                .then(left.package_hash.cmp(&right.package_hash))
        });
        activation_records.sort_by(|left, right| left.package_id.cmp(&right.package_id));
        Ok(ResolvedPackageSet {
            assets,
            activation_records,
        })
    }

    /// Resolve one explicit activation so bootstrap can quarantine a broken
    /// package without discarding unrelated healthy packages.
    pub fn resolve_activation(&self, activation: &ActivationRecord) -> Result<ResolvedPackageSet> {
        anyhow::ensure!(
            activation.enabled
                && activation.schema_version == 1
                && !activation.package_id.trim().is_empty(),
            "enabled activation record is invalid"
        );
        let package_id = activation.package_id.clone();
        let selected_hash = activation
            .current
            .as_deref()
            .context("enabled activation has no selected package hash")?;
        let installed = selected_installation(
            self.store.get_installed(&package_id)?,
            selected_hash,
            &package_id,
        )?;
        let package_root = self
            .store
            .package_path(selected_hash)?
            .canonicalize()
            .with_context(|| {
                format!(
                    "enabled package content is missing for '{package_id}' hash {selected_hash}"
                )
            })?;
        let mut assets = Vec::new();
        for asset in &installed.assets {
            let absolute_path = resolve_contained_asset(&package_root, &asset.path)
                .with_context(|| format!("resolving asset '{}'", asset.id))?;
            assets.push(ResolvedPackageAsset {
                package_id: installed.id.clone(),
                package_version: installed.version.clone(),
                package_hash: installed.hash.clone(),
                asset: asset.clone(),
                absolute_path,
            });
        }
        Ok(ResolvedPackageSet {
            assets,
            activation_records: vec![activation.clone()],
        })
    }
}

fn selected_installation(
    records: Vec<InstalledPackageRecord>,
    selected_hash: &str,
    package_id: &str,
) -> Result<InstalledPackageRecord> {
    records
        .into_iter()
        .find(|record| record.hash == selected_hash)
        .with_context(|| {
            format!(
                "enabled package '{package_id}' selected hash {selected_hash} has no installed projection"
            )
        })
}

fn resolve_contained_asset(package_root: &Path, declared_path: &str) -> Result<PathBuf> {
    let relative = Path::new(declared_path);
    anyhow::ensure!(
        !declared_path.trim().is_empty() && !relative.is_absolute(),
        "asset path must be nonempty and package-relative"
    );
    let absolute = package_root
        .join(relative)
        .canonicalize()
        .with_context(|| {
            format!(
                "declared package asset does not exist: {}",
                relative.display()
            )
        })?;
    anyhow::ensure!(
        absolute.starts_with(package_root),
        "declared package asset escapes its content-addressed root"
    );
    anyhow::ensure!(absolute.is_file(), "declared package asset is not a file");
    Ok(absolute)
}

fn asset_kind_rank(kind: AssetKind) -> u8 {
    match kind {
        AssetKind::AgentProfile => 0,
        AssetKind::Connector => 1,
        AssetKind::Hook => 2,
        AssetKind::Skill => 3,
        AssetKind::Executable => 4,
    }
}
