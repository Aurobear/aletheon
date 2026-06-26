pub mod writable_root;

pub use writable_root::{
    AccessMode, FileSystemSandboxPolicy, PathAccessError, PathAccessGuard, PathPattern,
    WritableRoot, PROTECTED_METADATA_NAMES,
};
