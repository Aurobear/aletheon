//! Linux FilesystemHost — operation-scoped file I/O.

use crate::error::{HostError, HostErrorKind};
use crate::filesystem::{
    AtomicWrite, EntryMetadata, FilesystemAccess, FilesystemHost, FilesystemScope, FsEventStream,
    RemoveFile, SymlinkPolicy, WriteReceipt,
};
use crate::path::HostPath;
use crate::receipt::HostReceipt;
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
static FORCE_OPENAT2_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub struct LinuxFilesystemHost {
    roots: Vec<RootHandle>,
    readable_paths: Vec<ReadableHandle>,
    access: FilesystemAccess,
    symlink_policy: SymlinkPolicy,
}

struct RootHandle {
    path: PathBuf,
    fd: OwnedFd,
}

struct ReadableHandle {
    path: PathBuf,
    fd: OwnedFd,
}

struct OpenedPath {
    display: PathBuf,
    fd: OwnedFd,
}

struct WriteTarget {
    parent_fd: OwnedFd,
    name: std::ffi::OsString,
}

impl WriteTarget {
    fn proc_path(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.parent_fd.as_raw_fd())).join(&self.name)
    }
}

impl LinuxFilesystemHost {
    pub fn scoped(scope: FilesystemScope) -> Result<Self, HostError> {
        if scope.roots.is_empty() && scope.readable_paths.is_empty() {
            return Err(HostError::new(
                HostErrorKind::PermissionDenied("empty filesystem scope".into()),
                "at least one admitted root is required",
            ));
        }
        let mut roots = Vec::with_capacity(scope.roots.len());
        for root in scope.roots {
            let canonical = std::fs::canonicalize(root.native())
                .map_err(|error| map_io_error(error, "resolve filesystem root"))?;
            if !canonical.is_dir() {
                return Err(HostError::new(
                    HostErrorKind::NotFound(canonical.display().to_string()),
                    "filesystem root is not a directory",
                ));
            }
            if !roots.iter().any(|root: &RootHandle| root.path == canonical) {
                let fd = open_directory(&canonical)?;
                roots.push(RootHandle {
                    path: canonical,
                    fd,
                });
            }
        }
        let mut readable_paths = Vec::with_capacity(scope.readable_paths.len());
        for path in scope.readable_paths {
            let canonical = std::fs::canonicalize(path.native())
                .map_err(|error| map_io_error(error, "resolve admitted read path"))?;
            if !canonical.is_file() {
                return Err(HostError::new(
                    HostErrorKind::NotFound(canonical.display().to_string()),
                    "admitted read path is not a file",
                ));
            }
            if !readable_paths
                .iter()
                .any(|readable: &ReadableHandle| readable.path == canonical)
            {
                let fd = open_path_absolute(&canonical, libc::O_RDONLY)?;
                readable_paths.push(ReadableHandle {
                    path: canonical,
                    fd,
                });
            }
        }
        Ok(Self {
            roots,
            readable_paths,
            access: scope.access,
            symlink_policy: scope.symlink_policy,
        })
    }

    fn candidate(&self, path: &HostPath) -> Result<(usize, PathBuf), HostError> {
        let raw = if path.native().is_absolute() {
            path.native().to_path_buf()
        } else {
            self.roots[0].path.join(path.native())
        };
        let normalized = normalize_lexically(&raw)?;
        self.roots
            .iter()
            .position(|root| normalized.starts_with(&root.path))
            .map(|index| (index, normalized))
            .ok_or_else(|| denied(path, "path is outside the admitted filesystem roots"))
    }

    fn resolve_existing(&self, path: &HostPath) -> Result<OpenedPath, HostError> {
        let raw = if path.native().is_absolute() {
            path.native().to_path_buf()
        } else if let Some(root) = self.roots.first() {
            root.path.join(path.native())
        } else {
            return Err(denied(path, "relative path has no admitted root"));
        };
        let normalized = normalize_lexically(&raw)?;
        if let Ok(canonical) = std::fs::canonicalize(&normalized) {
            if let Some(readable) = self
                .readable_paths
                .iter()
                .find(|readable| readable.path == canonical)
            {
                return Ok(OpenedPath {
                    display: canonical,
                    fd: readable
                        .fd
                        .try_clone()
                        .map_err(|error| map_io_error(error, "clone admitted read handle"))?,
                });
            }
        }
        let (root_index, candidate) = self.candidate(path)?;
        if self.symlink_policy == SymlinkPolicy::Deny {
            ensure_no_symlink(&self.roots[root_index].path, &candidate)?;
        }
        let canonical = std::fs::canonicalize(&candidate)
            .map_err(|error| map_io_error(error, "resolve filesystem path"))?;
        if !canonical.starts_with(&self.roots[root_index].path) {
            return Err(denied(path, "symlink resolves outside the admitted root"));
        }
        let relative = candidate
            .strip_prefix(&self.roots[root_index].path)
            .map_err(|_| denied(path, "path is outside root"))?;
        let fd = open_beneath(
            &self.roots[root_index].fd,
            relative,
            libc::O_RDONLY,
            self.symlink_policy == SymlinkPolicy::Deny,
        )?;
        Ok(OpenedPath {
            display: canonical,
            fd,
        })
    }

    fn resolve_write_target(&self, path: &HostPath) -> Result<WriteTarget, HostError> {
        if self.access != FilesystemAccess::ReadWrite {
            return Err(denied(path, "filesystem scope is read-only"));
        }
        let (root_index, candidate) = self.candidate(path)?;
        let parent = candidate
            .parent()
            .ok_or_else(|| denied(path, "write target has no parent"))?;
        if self.symlink_policy == SymlinkPolicy::Deny {
            ensure_no_symlink(&self.roots[root_index].path, parent)?;
            if candidate.exists() {
                ensure_no_symlink(&self.roots[root_index].path, &candidate)?;
            }
        }
        let canonical_parent = std::fs::canonicalize(parent)
            .map_err(|error| map_io_error(error, "resolve write parent"))?;
        if !canonical_parent.starts_with(&self.roots[root_index].path) {
            return Err(denied(
                path,
                "write parent resolves outside the admitted root",
            ));
        }
        let name = candidate
            .file_name()
            .ok_or_else(|| denied(path, "write target has no file name"))?;
        let target = canonical_parent.join(name);
        if target.exists() {
            let canonical_target = std::fs::canonicalize(&target)
                .map_err(|error| map_io_error(error, "resolve write target"))?;
            if !canonical_target.starts_with(&self.roots[root_index].path) {
                return Err(denied(
                    path,
                    "write target resolves outside the admitted root",
                ));
            }
        }
        let relative_parent = canonical_parent
            .strip_prefix(&self.roots[root_index].path)
            .map_err(|_| denied(path, "write parent is outside root"))?;
        let parent_fd = open_beneath(
            &self.roots[root_index].fd,
            relative_parent,
            libc::O_RDONLY | libc::O_DIRECTORY,
            self.symlink_policy == SymlinkPolicy::Deny,
        )?;
        Ok(WriteTarget {
            parent_fd,
            name: name.to_os_string(),
        })
    }
}

#[async_trait]
impl FilesystemHost for LinuxFilesystemHost {
    async fn metadata(&self, path: &HostPath) -> Result<EntryMetadata, HostError> {
        let resolved = self.resolve_existing(path)?;
        let meta = tokio::fs::metadata(proc_fd_path(&resolved.fd))
            .await
            .map_err(|error| map_io_error(error, "metadata"))?;
        Ok(EntryMetadata {
            path: HostPath::new(resolved.display),
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
            size_bytes: meta.len(),
            modified_unix_ms: meta
                .modified()
                .unwrap_or(std::time::UNIX_EPOCH)
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        })
    }

    async fn read(&self, path: &HostPath) -> Result<Vec<u8>, HostError> {
        let resolved = self.resolve_existing(path)?;
        tokio::fs::read(proc_fd_path(&resolved.fd))
            .await
            .map_err(|error| map_io_error(error, "read"))
    }

    async fn create_dir_all(&self, path: &HostPath) -> Result<HostReceipt, HostError> {
        let start = Instant::now();
        if self.access != FilesystemAccess::ReadWrite {
            return Err(denied(path, "filesystem scope is read-only"));
        }
        let (root_index, candidate) = self.candidate(path)?;
        let relative = candidate
            .strip_prefix(&self.roots[root_index].path)
            .map_err(|_| denied(path, "directory is outside the admitted root"))?;
        let mut current_fd = self.roots[root_index]
            .fd
            .try_clone()
            .map_err(|error| map_io_error(error, "clone filesystem root handle"))?;
        for component in relative.components() {
            let name = component.as_os_str();
            let relative_component = Path::new(name);
            let next = match open_beneath(
                &current_fd,
                relative_component,
                libc::O_PATH | libc::O_DIRECTORY,
                self.symlink_policy == SymlinkPolicy::Deny,
            ) {
                Ok(fd) => fd,
                Err(error) if matches!(error.kind, HostErrorKind::NotFound(_)) => {
                    mkdir_at(&current_fd, name)?;
                    open_beneath(
                        &current_fd,
                        relative_component,
                        libc::O_PATH | libc::O_DIRECTORY,
                        self.symlink_policy == SymlinkPolicy::Deny,
                    )?
                }
                Err(error) => return Err(error),
            };
            current_fd = next;
        }
        sync_directory(&current_fd)?;
        Ok(HostReceipt::ok(
            "create_dir_all",
            start.elapsed().as_micros() as u64,
        ))
    }

    async fn atomic_write(&self, request: AtomicWrite) -> Result<WriteReceipt, HostError> {
        let start = Instant::now();
        let target = self.resolve_write_target(&request.path)?;
        let target_path = target.proc_path();
        if let Some(expected) = &request.expected_sha256 {
            let existing_fd = open_beneath(
                &target.parent_fd,
                Path::new(&target.name),
                libc::O_RDONLY,
                self.symlink_policy == SymlinkPolicy::Deny,
            )
            .map_err(|error| {
                if matches!(error.kind, HostErrorKind::NotFound(_)) {
                    HostError::new(
                        HostErrorKind::Conflict("expected file does not exist".into()),
                        "atomic_write precondition",
                    )
                } else {
                    error
                }
            })?;
            let existing = tokio::fs::read(proc_fd_path(&existing_fd))
                .await
                .map_err(|error| map_io_error(error, "read write precondition"))?;
            let actual = format!("{:x}", Sha256::digest(&existing));
            if &actual != expected {
                return Err(HostError::new(
                    HostErrorKind::Conflict("stale workspace view".into()),
                    "atomic_write precondition",
                ));
            }
        }

        let parent = PathBuf::from(format!("/proc/self/fd/{}", target.parent_fd.as_raw_fd()));
        let name = target.name.to_string_lossy();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
        let result = async {
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            let mut file = options
                .open(&temporary)
                .await
                .map_err(|error| map_io_error(error, "create temporary file"))?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &request.content)
                .await
                .map_err(|error| map_io_error(error, "write temporary file"))?;
            if let Some(mode) = request.mode {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(mode))
                    .await
                    .map_err(|error| map_io_error(error, "set file mode"))?;
            }
            file.sync_all()
                .await
                .map_err(|error| map_io_error(error, "sync temporary file"))?;
            drop(file);
            tokio::fs::rename(&temporary, &target_path)
                .await
                .map_err(|error| map_io_error(error, "replace target"))?;
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| map_io_error(error, "sync parent directory"))?;
            Ok::<(), HostError>(())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        result?;

        Ok(WriteReceipt {
            bytes_written: request.content.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&request.content)),
            receipt: HostReceipt::ok("atomic_write", start.elapsed().as_micros() as u64),
        })
    }

    async fn remove_file(&self, request: RemoveFile) -> Result<HostReceipt, HostError> {
        let start = Instant::now();
        let path = &request.path;
        if self.access != FilesystemAccess::ReadWrite {
            return Err(denied(path, "filesystem scope is read-only"));
        }
        let target = self.resolve_existing(path)?;
        let target_path = proc_fd_path(&target.fd);
        let metadata = tokio::fs::metadata(&target_path)
            .await
            .map_err(|error| map_io_error(error, "metadata before remove"))?;
        if !metadata.is_file() {
            return Err(HostError::new(
                HostErrorKind::Conflict(target.display.display().to_string()),
                "remove target is not a regular file",
            ));
        }
        if let Some(expected) = &request.expected_sha256 {
            let existing = tokio::fs::read(&target_path)
                .await
                .map_err(|error| map_io_error(error, "read remove precondition"))?;
            let actual = format!("{:x}", Sha256::digest(&existing));
            if &actual != expected {
                return Err(HostError::new(
                    HostErrorKind::Conflict("stale workspace view".into()),
                    "remove_file precondition",
                ));
            }
        }
        let write_target = self.resolve_write_target(path)?;
        tokio::fs::remove_file(write_target.proc_path())
            .await
            .map_err(|error| map_io_error(error, "remove file"))?;
        std::fs::File::open(proc_fd_path(&write_target.parent_fd))
            .and_then(|directory| directory.sync_all())
            .map_err(|error| map_io_error(error, "sync parent directory"))?;
        Ok(HostReceipt::ok(
            "remove_file",
            start.elapsed().as_micros() as u64,
        ))
    }

    async fn watch(&self, root: &HostPath) -> Result<Box<dyn FsEventStream>, HostError> {
        self.resolve_existing(root)?;
        Err(HostError::unsupported(
            "filesystem watch is not implemented",
        ))
    }
}

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;

fn open_directory(path: &Path) -> Result<OwnedFd, HostError> {
    open_path_absolute(path, libc::O_RDONLY | libc::O_DIRECTORY)
}

fn open_path_absolute(path: &Path, flags: i32) -> Result<OwnedFd, HostError> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        HostError::new(
            HostErrorKind::PermissionDenied("NUL in path".into()),
            "open",
        )
    })?;
    // SAFETY: `path` is a live NUL-terminated string and successful `open`
    // returns a newly owned descriptor.
    let fd = unsafe { libc::open(path.as_ptr(), flags | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(map_io_error(std::io::Error::last_os_error(), "open path"));
    }
    // SAFETY: ownership of the newly-created descriptor transfers here.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn open_beneath(
    root: &OwnedFd,
    relative: &Path,
    flags: i32,
    deny_symlinks: bool,
) -> Result<OwnedFd, HostError> {
    #[cfg(test)]
    if FORCE_OPENAT2_UNAVAILABLE.load(Ordering::SeqCst) {
        return Err(map_io_error(
            std::io::Error::from_raw_os_error(libc::ENOSYS),
            "openat2 unavailable; scoped filesystem fails closed",
        ));
    }
    let relative = if relative.as_os_str().is_empty() {
        Path::new(".")
    } else {
        relative
    };
    let path = CString::new(relative.as_os_str().as_bytes()).map_err(|_| {
        HostError::new(
            HostErrorKind::PermissionDenied("NUL in path".into()),
            "openat2",
        )
    })?;
    let how = OpenHow {
        flags: (flags | libc::O_CLOEXEC) as u64,
        mode: 0,
        resolve: RESOLVE_BENEATH
            | RESOLVE_NO_MAGICLINKS
            | if deny_symlinks {
                RESOLVE_NO_SYMLINKS
            } else {
                0
            },
    };
    // SAFETY: arguments match Linux `openat2(2)` and point to live values for
    // the duration of the syscall. A successful return is a new descriptor.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root.as_raw_fd(),
            path.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        ) as i32
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        let operation = if error.raw_os_error() == Some(libc::ENOSYS) {
            "openat2 unavailable; scoped filesystem fails closed"
        } else {
            "openat2 scoped path"
        };
        return Err(map_io_error(error, operation));
    }
    // SAFETY: ownership of the newly-created descriptor transfers here.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn proc_fd_path(fd: &OwnedFd) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", fd.as_raw_fd()))
}

fn mkdir_at(parent: &OwnedFd, name: &std::ffi::OsStr) -> Result<(), HostError> {
    let name = CString::new(name.as_bytes()).map_err(|_| {
        HostError::new(
            HostErrorKind::PermissionDenied("NUL in directory name".into()),
            "mkdirat",
        )
    })?;
    // SAFETY: `name` is NUL-terminated and `parent` remains open for the call.
    let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o755) };
    if result < 0 {
        return Err(map_io_error(
            std::io::Error::last_os_error(),
            "create scoped directory",
        ));
    }
    Ok(())
}

fn sync_directory(fd: &OwnedFd) -> Result<(), HostError> {
    let directory = std::fs::File::open(proc_fd_path(fd))
        .map_err(|error| map_io_error(error, "open directory for sync"))?;
    directory
        .sync_all()
        .map_err(|error| map_io_error(error, "sync directory"))
}

fn normalize_lexically(path: &Path) -> Result<PathBuf, HostError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(HostError::new(
                        HostErrorKind::PermissionDenied(path.display().to_string()),
                        "path escapes its lexical root",
                    ));
                }
            }
        }
    }
    Ok(normalized)
}

fn ensure_no_symlink(root: &Path, target: &Path) -> Result<(), HostError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| denied(&HostPath::new(target.to_path_buf()), "path is outside root"))?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part.as_os_str());
        if current.exists()
            && std::fs::symlink_metadata(&current)
                .map_err(|error| map_io_error(error, "inspect symlink"))?
                .file_type()
                .is_symlink()
        {
            return Err(denied(
                &HostPath::new(target.to_path_buf()),
                "symlinks are denied by this filesystem scope",
            ));
        }
    }
    Ok(())
}

fn denied(path: &HostPath, detail: &str) -> HostError {
    HostError::new(
        HostErrorKind::PermissionDenied(path.logical().into()),
        detail,
    )
}

fn map_io_error(error: std::io::Error, operation: &str) -> HostError {
    let kind = match error.raw_os_error() {
        // `openat2` reports confinement violations as EXDEV and denied
        // symlink traversal as ELOOP. Both are policy denials, not ambient I/O.
        Some(libc::EXDEV) | Some(libc::ELOOP) => HostErrorKind::PermissionDenied(error.to_string()),
        _ => match error.kind() {
            std::io::ErrorKind::NotFound => HostErrorKind::NotFound(error.to_string()),
            std::io::ErrorKind::PermissionDenied => {
                HostErrorKind::PermissionDenied(error.to_string())
            }
            _ => HostErrorKind::Io(error.to_string()),
        },
    };
    HostError::new(kind, operation)
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static OPENAT2_TEST_LOCK: Mutex<()> = Mutex::new(());

    struct ForceOpenat2Unavailable;

    impl ForceOpenat2Unavailable {
        fn enable() -> Self {
            FORCE_OPENAT2_UNAVAILABLE.store(true, Ordering::SeqCst);
            Self
        }
    }

    impl Drop for ForceOpenat2Unavailable {
        fn drop(&mut self) {
            FORCE_OPENAT2_UNAVAILABLE.store(false, Ordering::SeqCst);
        }
    }

    fn host(root: &Path, access: FilesystemAccess) -> LinuxFilesystemHost {
        LinuxFilesystemHost::scoped(FilesystemScope {
            roots: vec![HostPath::new(root.to_path_buf())],
            readable_paths: vec![],
            access,
            symlink_policy: SymlinkPolicy::WithinRoot,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn read_write_round_trip() {
        let _lock = OPENAT2_TEST_LOCK.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let host = host(root.path(), FilesystemAccess::ReadWrite);
        let path = HostPath::new(root.path().join("test.txt"));
        let receipt = host
            .atomic_write(AtomicWrite {
                path: path.clone(),
                content: b"hello".to_vec(),
                expected_sha256: None,
                mode: Some(0o600),
            })
            .await
            .unwrap();
        assert_eq!(receipt.bytes_written, 5);
        assert_eq!(host.read(&path).await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn stale_write_rejected() {
        let _lock = OPENAT2_TEST_LOCK.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let host = host(root.path(), FilesystemAccess::ReadWrite);
        let path = HostPath::new(root.path().join("stale.txt"));
        tokio::fs::write(path.native(), b"original").await.unwrap();
        let error = host
            .atomic_write(AtomicWrite {
                path,
                content: b"new".to_vec(),
                expected_sha256: Some(format!("{:x}", Sha256::digest(b"wrong-hash"))),
                mode: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(error.kind, HostErrorKind::Conflict(_)));
    }

    #[tokio::test]
    async fn openat2_unavailable_fails_closed_without_fallback() {
        let _lock = OPENAT2_TEST_LOCK.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.txt");
        std::fs::write(&source, b"unchanged").unwrap();
        let host = host(root.path(), FilesystemAccess::ReadWrite);

        let forced = ForceOpenat2Unavailable::enable();
        let error = host
            .read(&HostPath::new(source.clone()))
            .await
            .expect_err("scoped reads must not fall back when openat2 is unavailable");
        drop(forced);

        assert!(matches!(error.kind, HostErrorKind::Io(_)));
        assert!(error.detail.contains("fails closed"));
        assert_eq!(std::fs::read(source).unwrap(), b"unchanged");
    }
}
