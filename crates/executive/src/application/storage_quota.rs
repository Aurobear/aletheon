//! Concurrent admission reservations for managed deployment storage.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageClass {
    Artifacts,
    Worktrees,
    Audit,
    Sessions,
    Google,
    GbrainSpool,
    Total,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageLimit {
    pub soft_bytes: u64,
    pub hard_bytes: u64,
    pub hard_items: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageUsage {
    pub bytes: u64,
    pub items: u64,
}

#[derive(Debug, Clone)]
pub struct StorageRoot {
    pub path: PathBuf,
    pub limit: StorageLimit,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QuotaError {
    #[error("storage class is not configured")]
    NotConfigured,
    #[error("storage hard limit exceeded for {class:?}")]
    HardLimit { class: StorageClass },
    #[error("managed storage contains a symlink or multiply-linked file")]
    UnsafeEntry,
    #[error("storage I/O failure: {0}")]
    Io(String),
}

#[derive(Debug, Default)]
struct ReservationState {
    next_id: u64,
    reservations: HashMap<u64, (StorageClass, StorageUsage)>,
}

#[derive(Debug)]
struct QuotaInner {
    roots: HashMap<StorageClass, StorageRoot>,
    state: Mutex<ReservationState>,
}

#[derive(Debug, Clone)]
pub struct StorageQuota(Arc<QuotaInner>);

impl StorageQuota {
    pub fn new(roots: HashMap<StorageClass, StorageRoot>) -> Result<Self, QuotaError> {
        for root in roots.values() {
            if root.limit.soft_bytes > root.limit.hard_bytes
                || root.limit.hard_bytes == 0
                || root.limit.hard_items == 0
            {
                return Err(QuotaError::HardLimit {
                    class: StorageClass::Total,
                });
            }
        }
        Ok(Self(Arc::new(QuotaInner {
            roots,
            state: Mutex::new(ReservationState::default()),
        })))
    }

    pub fn usage(&self, class: StorageClass) -> Result<StorageUsage, QuotaError> {
        let root = self.0.roots.get(&class).ok_or(QuotaError::NotConfigured)?;
        scan_managed_root(&root.path)
    }

    pub fn reserve(
        &self,
        class: StorageClass,
        expected_bytes: u64,
        expected_items: u64,
    ) -> Result<StorageReservation, QuotaError> {
        let root = self.0.roots.get(&class).ok_or(QuotaError::NotConfigured)?;
        let usage = scan_managed_root(&root.path)?;
        let mut state = self.0.state.lock().unwrap();
        let reserved = state
            .reservations
            .values()
            .filter(|(reserved_class, _)| *reserved_class == class)
            .fold(StorageUsage::default(), |sum, (_, item)| StorageUsage {
                bytes: sum.bytes.saturating_add(item.bytes),
                items: sum.items.saturating_add(item.items),
            });
        if usage
            .bytes
            .saturating_add(reserved.bytes)
            .saturating_add(expected_bytes)
            > root.limit.hard_bytes
            || usage
                .items
                .saturating_add(reserved.items)
                .saturating_add(expected_items)
                > root.limit.hard_items
        {
            return Err(QuotaError::HardLimit { class });
        }
        state.next_id = state.next_id.wrapping_add(1).max(1);
        let id = state.next_id;
        state.reservations.insert(
            id,
            (
                class,
                StorageUsage {
                    bytes: expected_bytes,
                    items: expected_items,
                },
            ),
        );
        Ok(StorageReservation {
            owner: Arc::downgrade(&self.0),
            id: Some(id),
        })
    }

    pub fn is_soft_limited(&self, class: StorageClass) -> Result<bool, QuotaError> {
        let root = self.0.roots.get(&class).ok_or(QuotaError::NotConfigured)?;
        Ok(self.usage(class)?.bytes >= root.limit.soft_bytes)
    }
}

#[derive(Debug)]
pub struct StorageReservation {
    owner: Weak<QuotaInner>,
    id: Option<u64>,
}

impl StorageReservation {
    /// Settle the in-process reservation after the operation has durably
    /// installed (or deduplicated) its managed file.
    pub fn commit(mut self) {
        self.release();
    }

    fn release(&mut self) {
        if let (Some(owner), Some(id)) = (self.owner.upgrade(), self.id.take()) {
            owner.state.lock().unwrap().reservations.remove(&id);
        }
    }
}

impl Drop for StorageReservation {
    fn drop(&mut self) {
        self.release();
    }
}

fn scan_managed_root(root: &Path) -> Result<StorageUsage, QuotaError> {
    if !root.exists() {
        return Ok(StorageUsage::default());
    }
    if fs::symlink_metadata(root)
        .map_err(io_error)?
        .file_type()
        .is_symlink()
    {
        return Err(QuotaError::UnsafeEntry);
    }
    let mut usage = StorageUsage::default();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
            if metadata.file_type().is_symlink() {
                return Err(QuotaError::UnsafeEntry);
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.nlink() > 1 {
                        return Err(QuotaError::UnsafeEntry);
                    }
                }
                usage.bytes = usage.bytes.saturating_add(metadata.len());
                usage.items = usage.items.saturating_add(1);
            }
        }
    }
    Ok(usage)
}

fn io_error(error: std::io::Error) -> QuotaError {
    QuotaError::Io(error.kind().to_string())
}
