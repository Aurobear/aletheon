//! Real FUSE mounting via fuse3.
//!
//! Provides `FuseMount` which manages the lifecycle of a FUSE mount backed by
//! `AgentFs`. All fuse3-dependent code is gated behind `#[cfg(feature = "fuse")]`.

use anyhow::Result;
use std::path::PathBuf;
use tracing::{info, warn};

use super::filesystem::AgentFs;

/// Manages a FUSE mount point for the agent virtual filesystem.
///
/// When the `fuse` feature is enabled, this wraps a real `fuse3` mount handle.
/// Without the feature, it operates in stub mode (always reports unmounted).
pub struct FuseMount {
    mount_point: PathBuf,
    #[cfg(feature = "fuse")]
    mount_handle: Option<fuse3::MountHandle>,
}

impl FuseMount {
    /// Mount the agent filesystem at the given path.
    ///
    /// Creates the mount point directory if it doesn't exist, then performs
    /// a real FUSE mount using fuse3's async `PathFileSystem` interface.
    #[cfg(feature = "fuse")]
    pub async fn mount(mount_point: PathBuf, fs: AgentFs) -> Result<Self> {
        use fuse3::path::Session;
        use fuse3::MountOptions;

        // Ensure mount point exists
        if !mount_point.exists() {
            std::fs::create_dir_all(&mount_point)?;
        }

        let mut mount_options = MountOptions::default();
        mount_options
            .fs_name("agent-fs".to_string())
            .allow_other(false)
            .auto_unmount(true);

        let session = Session::new(mount_options);
        let mount_handle = session.mount_with_unprivileged(fs, &mount_point).await?;

        info!(path = %mount_point.display(), "Agent FUSE filesystem mounted");

        Ok(Self {
            mount_point,
            mount_handle: Some(mount_handle),
        })
    }

    /// Mount stub when fuse feature is disabled.
    #[cfg(not(feature = "fuse"))]
    pub async fn mount(mount_point: PathBuf, _fs: AgentFs) -> Result<Self> {
        warn!(
            path = %mount_point.display(),
            "FUSE mount requested but 'fuse' feature is not enabled; operating in stub mode"
        );
        Ok(Self { mount_point })
    }

    /// Unmount the filesystem.
    #[cfg(feature = "fuse")]
    pub async fn unmount(&mut self) -> Result<()> {
        if let Some(handle) = self.mount_handle.take() {
            handle.unmount().await?;
            info!(path = %self.mount_point.display(), "Agent FUSE filesystem unmounted");
        }
        Ok(())
    }

    /// Unmount stub when fuse feature is disabled.
    #[cfg(not(feature = "fuse"))]
    pub async fn unmount(&mut self) -> Result<()> {
        info!("FUSE unmount (no-op in stub mode)");
        Ok(())
    }

    /// Check if the filesystem is currently mounted.
    #[cfg(feature = "fuse")]
    pub fn is_mounted(&self) -> bool {
        self.mount_handle.is_some()
    }

    /// Check if mounted (stub mode — always false).
    #[cfg(not(feature = "fuse"))]
    pub fn is_mounted(&self) -> bool {
        false
    }

    /// Get the mount point path.
    pub fn mount_point(&self) -> &PathBuf {
        &self.mount_point
    }
}

/// FUSE `PathFileSystem` implementation for `AgentFs`.
///
/// This bridges the in-memory `AgentFs` to the fuse3 async trait so it can
/// serve real FUSE requests. Only compiled when the `fuse` feature is active.
#[cfg(feature = "fuse")]
mod fuse_impl {
    use super::AgentFs;
    use async_trait::async_trait;
    use fuse3::path::reply::*;
    use fuse3::path::PathFileSystem;
    use fuse3::Errno;
    use std::ffi::OsStr;
    use std::num::NonZeroU32;

    /// Convert an anyhow error into a fuse3 Errno.
    fn to_errno(_e: anyhow::Error) -> Errno {
        Errno::from(libc::EIO)
    }

    #[async_trait]
    impl PathFileSystem for AgentFs {
        type DirEntryStream<'a> =
            Box<dyn futures::Stream<Item = Result<DirEntry, Errno>> + Send + 'a>;

        async fn init(&self) -> Result<(), Errno> {
            Ok(())
        }

        async fn destroy(&self) {}

        async fn lookup(&self, _parent: &OsStr, name: &OsStr) -> Result<ReplyEntry, Errno> {
            let name_str = name.to_string_lossy();
            let parent_path = _parent.to_string_lossy();
            let full_path = format!("{}/{}", parent_path.trim_end_matches('/'), name_str);

            let nodes = self.nodes.read().await;
            match nodes.get(&full_path) {
                Some(super::filesystem::FsNode::Directory { .. }) => Ok(ReplyEntry {
                    ttl: std::time::Duration::from_secs(1),
                    attr: libc::stat {
                        st_mode: libc::S_IFDIR | 0o755,
                        st_nlink: 2,
                        ..unsafe { std::mem::zeroed() }
                    },
                    ..Default::default()
                }),
                Some(super::filesystem::FsNode::File { content, .. }) => Ok(ReplyEntry {
                    ttl: std::time::Duration::from_secs(1),
                    attr: libc::stat {
                        st_mode: libc::S_IFREG | 0o644,
                        st_nlink: 1,
                        st_size: content.len() as i64,
                        ..unsafe { std::mem::zeroed() }
                    },
                    ..Default::default()
                }),
                Some(super::filesystem::FsNode::DynamicFile { .. }) => Ok(ReplyEntry {
                    ttl: std::time::Duration::from_secs(0),
                    attr: libc::stat {
                        st_mode: libc::S_IFREG | 0o644,
                        st_nlink: 1,
                        st_size: 0,
                        ..unsafe { std::mem::zeroed() }
                    },
                    ..Default::default()
                }),
                None => Err(Errno::from(libc::ENOENT)),
            }
        }

        async fn getattr(&self, path: &OsStr, _fh: Option<u64>) -> Result<ReplyAttr, Errno> {
            let path_str = path.to_string_lossy().to_string();
            let normalized = if path_str == "/" {
                "/".to_string()
            } else {
                path_str.trim_end_matches('/').to_string()
            };

            let nodes = self.nodes.read().await;
            match nodes.get(&normalized) {
                Some(super::filesystem::FsNode::Directory { .. }) => Ok(ReplyAttr {
                    ttl: std::time::Duration::from_secs(1),
                    attr: libc::stat {
                        st_mode: libc::S_IFDIR | 0o755,
                        st_nlink: 2,
                        ..unsafe { std::mem::zeroed() }
                    },
                }),
                Some(super::filesystem::FsNode::File { content, .. }) => Ok(ReplyAttr {
                    ttl: std::time::Duration::from_secs(1),
                    attr: libc::stat {
                        st_mode: libc::S_IFREG | 0o644,
                        st_nlink: 1,
                        st_size: content.len() as i64,
                        ..unsafe { std::mem::zeroed() }
                    },
                }),
                Some(super::filesystem::FsNode::DynamicFile { .. }) => Ok(ReplyAttr {
                    ttl: std::time::Duration::from_secs(0),
                    attr: libc::stat {
                        st_mode: libc::S_IFREG | 0o644,
                        st_nlink: 1,
                        st_size: 0,
                        ..unsafe { std::mem::zeroed() }
                    },
                }),
                None => Err(Errno::from(libc::ENOENT)),
            }
        }

        async fn read(
            &self,
            path: &OsStr,
            _fh: u64,
            offset: u64,
            size: u32,
        ) -> Result<ReplyData, Errno> {
            let path_str = path.to_string_lossy().to_string();
            let data = self.read(&path_str).await.map_err(to_errno)?;

            let start = offset as usize;
            let end = (start + size as usize).min(data.len());
            if start >= data.len() {
                return Ok(ReplyData {
                    data: Default::default(),
                });
            }
            Ok(ReplyData {
                data: data[start..end].to_vec().into(),
            })
        }

        async fn write(
            &self,
            path: &OsStr,
            _fh: u64,
            offset: u64,
            data: &[u8],
            _write_flags: u32,
            _flags: u32,
        ) -> Result<ReplyWrite, Errno> {
            let path_str = path.to_string_lossy().to_string();
            // For simplicity, replace content entirely (offset ignored for virtual FS)
            self.write(&path_str, data).await.map_err(to_errno)?;
            Ok(ReplyWrite {
                written: data.len() as u32,
            })
        }

        async fn readdir(
            &self,
            path: &OsStr,
            _fh: u64,
            offset: u64,
        ) -> Result<ReplyDirectory<Self::DirEntryStream<'_>>, Errno> {
            let path_str = path.to_string_lossy().to_string();
            let entries = self.readdir(&path_str).await.map_err(to_errno)?;

            let stream =
                futures::stream::iter(entries.into_iter().skip(offset as usize).enumerate().map(
                    move |(i, name)| {
                        Ok(DirEntry {
                            name: name.into(),
                            kind: libc::DT_DIR,
                            offset: (offset + i as u64 + 1) as i64,
                        })
                    },
                ));

            Ok(ReplyDirectory {
                entries: Box::new(stream),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;
    use std::sync::Arc;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    #[tokio::test]
    async fn test_fuse_mount_stub_mode() {
        let mount_point = std::env::temp_dir().join("test-agent-fuse-mount");
        let clock = test_clock();
        let fs = AgentFs::new(mount_point.clone(), clock);
        let mut mount = FuseMount::mount(mount_point.clone(), fs).await.unwrap();

        // In stub mode (without fuse feature), is_mounted returns false
        #[cfg(not(feature = "fuse"))]
        assert!(!mount.is_mounted());

        // With fuse feature, it would be mounted
        #[cfg(feature = "fuse")]
        assert!(mount.is_mounted());

        mount.unmount().await.unwrap();
        assert!(!mount.is_mounted());

        // Clean up
        let _ = std::fs::remove_dir_all(&mount_point);
    }

    #[tokio::test]
    async fn test_fuse_mount_point_accessor() {
        let mount_point = std::env::temp_dir().join("test-agent-fuse-accessor");
        let clock = test_clock();
        let fs = AgentFs::new(mount_point.clone(), clock);
        let mount = FuseMount::mount(mount_point.clone(), fs).await.unwrap();

        assert_eq!(mount.mount_point(), &mount_point);
    }

    #[tokio::test]
    async fn test_fuse_unmount_idempotent() {
        let mount_point = std::env::temp_dir().join("test-agent-fuse-idempotent");
        let clock = test_clock();
        let fs = AgentFs::new(mount_point.clone(), clock);
        let mut mount = FuseMount::mount(mount_point.clone(), fs).await.unwrap();

        // First unmount
        mount.unmount().await.unwrap();
        assert!(!mount.is_mounted());

        // Second unmount should succeed (no-op)
        mount.unmount().await.unwrap();
        assert!(!mount.is_mounted());
    }
}
