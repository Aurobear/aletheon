//! Package archive inspector.
//!
//! Opens .tar.gz packages, validates structure, verifies checksums,
//! and extracts to a staging directory.

use anyhow::{bail, Context, Result};
use fabric::{AssetKind, PackageManifest};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use tar::Archive;

use super::manifest;
use super::validation;

/// Result of inspecting a package archive.
#[derive(Debug)]
pub struct InspectionResult {
    pub manifest: PackageManifest,
    pub package_hash: String,
    pub file_count: usize,
    pub total_size: u64,
}

/// Inspect a .tar.gz package file.
///
/// Validates:
/// - extension.toml exists and parses correctly
/// - checksums.sha256 exists and covers all files
/// - All entry paths are safe (no traversal, absolute, etc.)
/// - All file sizes within limits
/// - All hashes match declared checksums
///
/// Does NOT extract to disk — returns raw file data for staging.
pub fn inspect_package(package_path: &Path) -> Result<InspectionResult> {
    // First pass: count entries and validate file count before allocating.
    {
        let file = std::fs::File::open(package_path)
            .with_context(|| format!("cannot open package: {}", package_path.display()))?;
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);
        let count = archive
            .entries()
            .context("failed to read archive entries")?
            .filter_map(|entry| entry.ok())
            .count();
        validation::validate_file_count(count)?;
    }

    // Second pass: read all file data into memory, validate paths and sizes.
    let file = std::fs::File::open(package_path)
        .with_context(|| format!("cannot open package: {}", package_path.display()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    let mut total_size: u64 = 0;

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let entry_path = entry
            .path()
            .with_context(|| "invalid entry path encoding")?;

        // Only allow regular files and directories. Reject symlinks,
        // hardlinks, block/char devices, FIFOs, and unknown types.
        let entry_type = entry.header().entry_type();
        match entry_type {
            tar::EntryType::Directory => continue,
            tar::EntryType::Regular => {}
            _ => bail!(
                "unsupported entry type {:?} for path: {}",
                entry_type,
                entry_path.display()
            ),
        }

        validation::validate_entry_path(&entry_path)?;

        let path_str = entry_path.to_string_lossy().to_string();

        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .with_context(|| format!("failed to read entry: {}", path_str))?;
        let size = data.len() as u64;

        validation::validate_file_size(size, &path_str)?;
        total_size += size;
        validation::validate_total_size(total_size)?;

        // Reject duplicate entry paths in the same archive.
        if files.contains_key(&path_str) {
            bail!("duplicate entry path in archive: {}", path_str);
        }
        files.insert(path_str, data);
    }

    // Verify extension.toml exists.
    let manifest_bytes = files
        .get("extension.toml")
        .context("extension.toml not found in package")?;
    let manifest_content =
        std::str::from_utf8(manifest_bytes).context("extension.toml is not valid UTF-8")?;
    let manifest = manifest::parse_package_manifest(manifest_content)?;

    // Verify checksums.sha256 exists and is valid.
    let checksum_bytes = files
        .get("checksums.sha256")
        .context("checksums.sha256 not found in package")?;
    let checksum_content =
        std::str::from_utf8(checksum_bytes).context("checksums.sha256 is not valid UTF-8")?;
    let checksums = manifest::parse_checksums(checksum_content)?;

    // Verify all checksums match.  Exclude checksums.sha256 itself from the
    // "every file must have a checksum" direction — the checksum file cannot
    // contain its own hash (chicken-and-egg).
    let mut files_for_verification = files.clone();
    files_for_verification.remove("checksums.sha256");
    validation::verify_all_checksums(&files_for_verification, &checksums)?;
    validate_declared_assets(&manifest, &files)?;

    // Compute package content hash (hash of all file paths + data, sorted).
    let mut hasher = Sha256::new();
    let mut sorted_paths: Vec<&String> = files.keys().collect();
    sorted_paths.sort();
    for path in sorted_paths {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(&files[path]);
        hasher.update(b"\0");
    }
    let package_hash = format!("{:x}", hasher.finalize());

    Ok(InspectionResult {
        manifest,
        package_hash,
        file_count: files.len(),
        total_size,
    })
}

fn validate_declared_assets(
    manifest: &PackageManifest,
    files: &HashMap<String, Vec<u8>>,
) -> Result<()> {
    for asset in &manifest.assets {
        validation::validate_entry_path(Path::new(&asset.path))?;
        let (prefix, suffix) = match asset.kind {
            AssetKind::Skill => ("assets/skills/", Some("/SKILL.md")),
            AssetKind::Hook => ("assets/hooks/", Some(".toml")),
            AssetKind::AgentProfile => ("assets/agents/", Some(".md")),
            AssetKind::Connector => ("assets/connectors/", None),
            AssetKind::Executable => ("assets/executables/", None),
        };
        if !asset.path.starts_with(prefix)
            || suffix.is_some_and(|expected| !asset.path.ends_with(expected))
        {
            bail!(
                "asset kind/path mismatch for '{}': {}",
                asset.id,
                asset.path
            );
        }
        if !files.contains_key(&asset.path) {
            bail!("declared asset file is missing: {}", asset.path);
        }
        if asset.kind == AssetKind::Executable {
            let content = std::str::from_utf8(&files[&asset.path])
                .context("runtime manifest is not valid UTF-8")?;
            let runtime = manifest::parse_executable_runtime_manifest(content)?;
            if !files.contains_key(&runtime.command) {
                bail!("runtime command is missing from package: {}", runtime.command);
            }
        }
    }
    Ok(())
}

/// Extract inspected package to a staging directory.
///
/// First validates the package via [`inspect_package`], then extracts
/// the full archive tree into `staging_dir`. Returns the same
/// [`InspectionResult`] produced by the inspection pass.
pub fn extract_to_staging(
    package_path: &Path,
    staging_dir: &Path,
) -> Result<InspectionResult> {
    // Validate everything first.
    let result = inspect_package(package_path)?;

    // Extract to staging.
    let file = std::fs::File::open(package_path)
        .with_context(|| format!("cannot open package: {}", package_path.display()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    std::fs::create_dir_all(staging_dir)
        .with_context(|| format!("cannot create staging dir: {}", staging_dir.display()))?;

    // Canonicalize staging_dir for path-traversal defense.
    let canonical_staging = staging_dir
        .canonicalize()
        .with_context(|| format!("cannot canonicalize staging dir: {}", staging_dir.display()))?;

    // Wrap extraction in a fallible block so we can clean up on failure.
    let extract_result = (|| -> Result<()> {
        for entry_result in archive.entries()? {
            let mut entry = entry_result?;
            let entry_path = entry.path()?.to_path_buf();

            // Re-validate for defense-in-depth.
            validation::validate_entry_path(&entry_path)?;

            // Only extract regular files. Directories are created implicitly
            // by parent creation below; everything else is rejected.
            let entry_type = entry.header().entry_type();
            match entry_type {
                tar::EntryType::Directory => continue,
                tar::EntryType::Regular => {}
                _ => bail!(
                    "unsupported entry type {:?} in extract: {}",
                    entry_type,
                    entry_path.display()
                ),
            }

            let dest = staging_dir.join(&entry_path);

            // Verify the resolved destination is within staging_dir.
            {
                let dest_abs = match dest.canonicalize() {
                    Ok(p) => p,
                    Err(_) => {
                        // File does not exist yet; canonicalize the deepest
                        // existing ancestor and append the remainder.
                        let mut anc = dest
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| Path::new(".").to_path_buf());
                        while !anc.exists() {
                            anc = anc
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or_else(|| Path::new(".").to_path_buf());
                        }
                        let remainder = dest
                            .strip_prefix(&anc)
                            .unwrap_or_else(|_| Path::new(""));
                        anc.canonicalize()?.join(remainder)
                    }
                };
                if !dest_abs.starts_with(&canonical_staging) {
                    bail!(
                        "entry escapes staging directory: {}",
                        entry_path.display()
                    );
                }
            }

            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create parent directory: {}", parent.display()))?;
            }
            entry
                .unpack(&dest)
                .with_context(|| format!("cannot unpack entry to: {}", dest.display()))?;
        }
        Ok(())
    })();

    if let Err(e) = extract_result {
        let _ = std::fs::remove_dir_all(staging_dir);
        return Err(e);
    }

    Ok(result)
}

/// Compute the SHA-256 content hash of all files in the given map.
///
/// Uses the same stable algorithm as [`inspect_package`] — sorted
/// path-then-data with null separators — so the result can be compared
/// against the hash stored in an [`InspectionResult`].
pub fn compute_files_hash(files: &HashMap<String, Vec<u8>>) -> String {
    let mut hasher = Sha256::new();
    let mut sorted_paths: Vec<&String> = files.keys().collect();
    sorted_paths.sort();
    for path in sorted_paths {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(&files[path]);
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_package(dir: &Path, name: &str) -> PathBuf {
        let pkg_dir = dir.join(name);
        std::fs::create_dir_all(pkg_dir.join("assets/skills/demo")).unwrap();

        let extension_toml = r#"
schema_version = 1

[package]
id = "test.minimal"
version = "0.1.0"
description = "A test package"
compatibility = { min_aletheon = "0.1.0" }

[[assets]]
kind = "skill"
id = "skill.demo"
path = "assets/skills/demo/SKILL.md"
"#;
        std::fs::write(pkg_dir.join("extension.toml"), extension_toml).unwrap();

        std::fs::write(
            pkg_dir.join("assets/skills/demo/SKILL.md"),
            "---\nname: demo-skill\ndescription: A demo skill\n---\n# Demo\n",
        )
        .unwrap();

        // Generate checksums covering all files in the package directory.
        let mut checksum_lines: Vec<String> = Vec::new();
        for walk_entry in walkdir::WalkDir::new(&pkg_dir)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let rel = walk_entry.path().strip_prefix(&pkg_dir).unwrap();
            if rel.as_os_str().is_empty() {
                continue;
            }
            if walk_entry.file_type().is_dir() {
                continue;
            }
            let data = std::fs::read(walk_entry.path()).unwrap();
            let hash = format!("{:x}", Sha256::digest(&data));
            checksum_lines.push(format!("{}  {}", hash, rel.display()));
        }
        let checksums_content = checksum_lines.join("\n") + "\n";
        std::fs::write(pkg_dir.join("checksums.sha256"), &checksums_content).unwrap();

        // Create tar.gz.
        let tar_path = dir.join(format!("{}.tar.gz", name));
        let tar_file = std::fs::File::create(&tar_path).unwrap();
        let encoder = GzEncoder::new(tar_file, Compression::default());
        let mut tar_builder = tar::Builder::new(encoder);

        for walk_entry in walkdir::WalkDir::new(&pkg_dir)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let rel = walk_entry.path().strip_prefix(&pkg_dir).unwrap();
            if rel.as_os_str().is_empty() {
                continue;
            }
            if walk_entry.file_type().is_dir() {
                tar_builder.append_dir(rel, walk_entry.path()).unwrap();
            } else {
                tar_builder
                    .append_path_with_name(walk_entry.path(), rel)
                    .unwrap();
            }
        }
        let encoder = tar_builder.into_inner().unwrap();
        encoder.finish().unwrap();

        tar_path
    }

    fn create_package_with_special_entry(dir: &Path, entry_type: tar::EntryType) -> PathBuf {
        let path = dir.join("special.tar.gz");
        let file = std::fs::File::create(&path).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_path("assets/special").unwrap();
        header.set_entry_type(entry_type);
        header.set_size(0);
        if entry_type == tar::EntryType::Link {
            header.set_link_name("extension.toml").unwrap();
        }
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        path
    }

    #[test]
    fn inspect_valid_package() {
        let tmp = TempDir::new().unwrap();
        let tar_path = create_test_package(tmp.path(), "valid");
        let result = inspect_package(&tar_path).unwrap();
        assert_eq!(result.manifest.package.id.0, "test.minimal");
        // extension.toml, checksums.sha256, SKILL.md
        assert_eq!(result.file_count, 3);
        assert!(result.total_size > 0);
        assert!(!result.package_hash.is_empty());
    }

    #[test]
    fn inspect_rejects_hardlink_fifo_and_device_entries() {
        for entry_type in [
            tar::EntryType::Link,
            tar::EntryType::Fifo,
            tar::EntryType::Block,
            tar::EntryType::Char,
        ] {
            let temp = TempDir::new().unwrap();
            let package = create_package_with_special_entry(temp.path(), entry_type);
            let error = inspect_package(&package).unwrap_err().to_string();
            assert!(
                error.contains("unsupported entry type"),
                "unexpected error for {entry_type:?}: {error}"
            );
        }
    }

    #[test]
    fn inspect_rejects_missing_extension_toml() {
        let tmp = TempDir::new().unwrap();
        let tar_path = tmp.path().join("bad.tar.gz");
        let tar_file = std::fs::File::create(&tar_path).unwrap();
        let encoder = GzEncoder::new(tar_file, Compression::default());
        let mut tar_builder = tar::Builder::new(encoder);
        // Only add checksums, no extension.toml.
        let mut header = tar::Header::new_gnu();
        header.set_path("checksums.sha256").unwrap();
        header.set_size(0);
        header.set_cksum();
        tar_builder.append(&header, &[][..]).unwrap();
        let encoder = tar_builder.into_inner().unwrap();
        encoder.finish().unwrap();

        assert!(inspect_package(&tar_path).is_err());
    }

    #[test]
    fn inspect_rejects_checksum_mismatch() {
        let tmp = TempDir::new().unwrap();
        let tar_path = tmp.path().join("bad-checksum.tar.gz");
        let tar_file = std::fs::File::create(&tar_path).unwrap();
        let encoder = GzEncoder::new(tar_file, Compression::default());
        let mut tar_builder = tar::Builder::new(encoder);

        // Add extension.toml with minimal valid content
        let toml = r#"
schema_version = 1

[package]
id = "test.bad"
version = "0.1.0"
description = "bad checksum test"

[[assets]]
kind = "skill"
id = "s"
path = "s.md"
"#;
        let mut toml_header = tar::Header::new_gnu();
        toml_header.set_path("extension.toml").unwrap();
        toml_header.set_size(toml.len() as u64);
        toml_header.set_cksum();
        tar_builder.append(&toml_header, toml.as_bytes()).unwrap();

        // Add checksums.sha256 with wrong hash
        let bad_checksum = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  extension.toml\n";
        let mut cs_header = tar::Header::new_gnu();
        cs_header.set_path("checksums.sha256").unwrap();
        cs_header.set_size(bad_checksum.len() as u64);
        cs_header.set_cksum();
        tar_builder.append(&cs_header, bad_checksum.as_bytes()).unwrap();

        let encoder = tar_builder.into_inner().unwrap();
        encoder.finish().unwrap();

        let err = inspect_package(&tar_path).unwrap_err().to_string();
        assert!(err.contains("checksum"), "expected checksum error, got: {}", err);
    }

    #[test]
    fn extract_to_staging_works() {
        let tmp = TempDir::new().unwrap();
        let tar_path = create_test_package(tmp.path(), "staging-test");
        let staging = tmp.path().join("staging");
        let result = extract_to_staging(&tar_path, &staging).unwrap();

        assert_eq!(result.manifest.package.id.0, "test.minimal");
        assert!(staging.join("extension.toml").exists());
        assert!(staging
            .join("assets/skills/demo/SKILL.md")
            .exists());
        assert!(staging.join("checksums.sha256").exists());
    }

    #[test]
    fn compute_files_hash_deterministic() {
        let mut files = HashMap::new();
        files.insert("b.txt".to_string(), b"beta".to_vec());
        files.insert("a.txt".to_string(), b"alpha".to_vec());

        let h1 = compute_files_hash(&files);
        let h2 = compute_files_hash(&files);
        assert_eq!(h1, h2);

        // Different data produces different hash.
        files.insert("c.txt".to_string(), b"gamma".to_vec());
        let h3 = compute_files_hash(&files);
        assert_ne!(h1, h3);
    }

    #[test]
    fn inspect_rejects_symlink_entry() {
        let tmp = TempDir::new().unwrap();
        let tar_path = tmp.path().join("symlink.tar.gz");
        let tar_file = std::fs::File::create(&tar_path).unwrap();
        let encoder = GzEncoder::new(tar_file, Compression::default());
        let mut tar_builder = tar::Builder::new(encoder);

        // Add a valid extension.toml so the archive has the required file.
        let toml = r#"
schema_version = 1

[package]
id = "test.sym"
version = "0.1.0"
description = "symlink test"

[[assets]]
kind = "skill"
id = "s"
path = "s.md"
"#;
        let mut toml_header = tar::Header::new_gnu();
        toml_header.set_path("extension.toml").unwrap();
        toml_header.set_size(toml.len() as u64);
        toml_header.set_cksum();
        tar_builder.append(&toml_header, toml.as_bytes()).unwrap();

        // Add a regular file with its checksum.
        let skill_data = b"# demo\n";
        let mut skill_header = tar::Header::new_gnu();
        skill_header.set_path("s.md").unwrap();
        skill_header.set_size(skill_data.len() as u64);
        skill_header.set_cksum();
        tar_builder.append(&skill_header, &skill_data[..]).unwrap();

        // Add checksums.sha256 with correct hashes.
        let toml_hash = format!("{:x}", Sha256::digest(toml.as_bytes()));
        let skill_hash = format!("{:x}", Sha256::digest(skill_data));
        let checksums = format!(
            "{}  extension.toml\n{}  s.md\n",
            toml_hash, skill_hash
        );
        let mut cs_header = tar::Header::new_gnu();
        cs_header.set_path("checksums.sha256").unwrap();
        cs_header.set_size(checksums.len() as u64);
        cs_header.set_cksum();
        tar_builder.append(&cs_header, checksums.as_bytes()).unwrap();

        // Add a symlink entry — this must be rejected.
        let mut symlink_header = tar::Header::new_gnu();
        symlink_header.set_entry_type(tar::EntryType::Symlink);
        symlink_header.set_path("evil-link").unwrap();
        symlink_header.set_link_name("extension.toml").unwrap();
        symlink_header.set_size(0);
        symlink_header.set_cksum();
        tar_builder.append(&symlink_header, &[][..]).unwrap();

        let encoder = tar_builder.into_inner().unwrap();
        encoder.finish().unwrap();

        let err = inspect_package(&tar_path).unwrap_err().to_string();
        assert!(
            err.contains("unsupported entry type"),
            "expected unsupported entry type error, got: {}",
            err
        );
    }

    #[test]
    fn inspect_rejects_duplicate_paths() {
        let tmp = TempDir::new().unwrap();
        let tar_path = tmp.path().join("duplicate.tar.gz");
        let tar_file = std::fs::File::create(&tar_path).unwrap();
        let encoder = GzEncoder::new(tar_file, Compression::default());
        let mut tar_builder = tar::Builder::new(encoder);

        // Add extension.toml twice with the same path.
        let toml = r#"
schema_version = 1

[package]
id = "test.dup"
version = "0.1.0"
description = "duplicate test"

[[assets]]
kind = "skill"
id = "s"
path = "s.md"
"#;
        let mut h1 = tar::Header::new_gnu();
        h1.set_path("extension.toml").unwrap();
        h1.set_size(toml.len() as u64);
        h1.set_cksum();
        tar_builder.append(&h1, toml.as_bytes()).unwrap();

        let mut h2 = tar::Header::new_gnu();
        h2.set_path("extension.toml").unwrap();
        h2.set_size(toml.len() as u64);
        h2.set_cksum();
        tar_builder.append(&h2, toml.as_bytes()).unwrap();

        let encoder = tar_builder.into_inner().unwrap();
        encoder.finish().unwrap();

        let err = inspect_package(&tar_path).unwrap_err().to_string();
        assert!(
            err.contains("duplicate entry path"),
            "expected duplicate entry error, got: {}",
            err
        );
    }
}
