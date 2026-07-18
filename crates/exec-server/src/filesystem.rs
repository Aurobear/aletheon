//! Workspace-confined filesystem operations for exec-server.

use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::Deserialize;

use crate::protocol::*;

const MAX_READ_BYTES: u64 = 100 * 1024 * 1024;
const MAX_FILE_HANDLES: usize = 128;
const MAX_CHUNK_BYTES: usize = 1024 * 1024;

pub struct WorkspaceRoots {
    roots: Vec<PathBuf>,
}
impl WorkspaceRoots {
    pub fn from_env() -> std::io::Result<Self> {
        let encoded = std::env::var("ALETHEON_EXEC_SERVER_WORKSPACE_ROOTS").map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "ALETHEON_EXEC_SERVER_WORKSPACE_ROOTS must be set",
            )
        })?;
        let configured: Vec<PathBuf> = serde_json::from_str(&encoded).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid exec-server workspace roots: {e}"),
            )
        })?;
        Self::new(configured).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
    }
    fn new(configured: Vec<PathBuf>) -> Result<Self, String> {
        if configured.is_empty() {
            return Err("exec-server requires at least one workspace root".into());
        }
        let roots = configured
            .into_iter()
            .map(|r| {
                r.canonicalize()
                    .map_err(|e| format!("workspace root {} is unavailable: {e}", r.display()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if roots.iter().any(|r| !r.is_dir()) {
            return Err("exec-server workspace roots must be directories".into());
        }
        Ok(Self { roots })
    }
    fn existing_path(&self, path: &Path) -> Result<PathBuf, String> {
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("path {} is unavailable: {e}", path.display()))?;
        self.ensure_contained(canonical)
    }

    pub fn execution_dir(&self, requested: Option<&Path>) -> Result<PathBuf, String> {
        let path = requested.unwrap_or(&self.roots[0]);
        let canonical = self.existing_path(path)?;
        if !canonical.is_dir() {
            return Err("process working_dir must be a directory".into());
        }
        Ok(canonical)
    }
    fn write_path(&self, path: &Path) -> Result<PathBuf, String> {
        if path.exists() {
            return self.existing_path(path);
        }
        let name = path
            .file_name()
            .ok_or_else(|| "write path must name an entry".to_string())?;
        let parent = path
            .parent()
            .ok_or_else(|| "write path must have a parent".to_string())?
            .canonicalize()
            .map_err(|e| format!("write parent is unavailable: {e}"))?;
        self.ensure_contained(parent.join(name))
    }
    fn prospective_write_path(&self, path: &Path) -> Result<PathBuf, String> {
        if path.exists() || path.parent().is_some_and(Path::exists) {
            return self.write_path(path);
        }
        let mut ancestor = path;
        let mut missing = Vec::new();
        while !ancestor.exists() {
            let name = ancestor
                .file_name()
                .ok_or_else(|| "patch target must name an entry".to_string())?;
            missing.push(name.to_os_string());
            ancestor = ancestor
                .parent()
                .ok_or_else(|| "patch target has no existing ancestor".to_string())?;
        }
        let mut resolved = self.existing_path(ancestor)?;
        for component in missing.iter().rev() {
            resolved.push(component);
        }
        self.ensure_contained(resolved)
    }
    fn ensure_contained(&self, canonical: PathBuf) -> Result<PathBuf, String> {
        self.roots
            .iter()
            .any(|r| canonical.starts_with(r))
            .then_some(canonical)
            .ok_or_else(|| "path is outside configured workspace roots".into())
    }
}

struct OpenFile {
    file: std::fs::File,
}
pub struct FileManager {
    inner: Mutex<FileManagerInner>,
}
struct FileManagerInner {
    next: u64,
    files: HashMap<String, OpenFile>,
}
impl Default for FileManager {
    fn default() -> Self {
        Self {
            inner: Mutex::new(FileManagerInner {
                next: 1,
                files: HashMap::new(),
            }),
        }
    }
}

#[derive(Deserialize)]
struct PathParams {
    path: PathBuf,
}
#[derive(Deserialize)]
struct WriteParams {
    path: PathBuf,
    content: String,
    #[serde(default)]
    deny_exact: Vec<PathBuf>,
    #[serde(default)]
    write_roots: Option<Vec<PathBuf>>,
}
#[derive(Deserialize)]
struct ApplyPatchParams {
    patch: String,
    #[serde(default)]
    working_dir: Option<PathBuf>,
    #[serde(default)]
    deny_exact: Vec<PathBuf>,
    #[serde(default)]
    write_roots: Option<Vec<PathBuf>>,
}
#[derive(Deserialize)]
struct ReadChunkParams {
    handle: String,
    offset: u64,
    size: usize,
}
#[derive(Deserialize)]
struct WalkParams {
    path: PathBuf,
    max_depth: usize,
}
#[derive(Deserialize)]
struct CopyParams {
    source: PathBuf,
    dest: PathBuf,
}
#[derive(Deserialize)]
struct OpenParams {
    path: PathBuf,
    mode: String,
}
#[derive(Deserialize)]
struct HandleParams {
    handle: String,
}

pub fn handle_fs(
    method: &str,
    params: &serde_json::Value,
    workspace: &WorkspaceRoots,
    files: &FileManager,
) -> Option<Response> {
    let result = match method {
        "fs/read" => read(params, workspace),
        "fs/readChunk" => read_chunk(params, files),
        "fs/write" => write(params, workspace),
        "fs/applyPatch" => apply_patch(params, workspace),
        "fs/list" => list(params, workspace),
        "fs/metadata" => metadata(params, workspace),
        "fs/walk" => walk(params, workspace),
        "fs/remove" => remove(params, workspace),
        "fs/copy" => copy(params, workspace),
        "fs/open" => open(params, workspace, files),
        "fs/close" => close(params, files),
        method if method.starts_with("fs/") => Err((
            METHOD_NOT_FOUND,
            format!("Filesystem method not implemented: {method}"),
        )),
        _ => return None,
    };
    Some(match result {
        Ok(v) => Response::ok(serde_json::Value::Null, v),
        Err((c, m)) => Response::err(serde_json::Value::Null, c, m),
    })
}
fn params<T: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
    method: &str,
) -> Result<T, (i32, String)> {
    serde_json::from_value(value.clone())
        .map_err(|e| (INVALID_PARAMS, format!("Invalid {method} params: {e}")))
}
fn access<T>(r: Result<T, String>) -> Result<T, (i32, String)> {
    r.map_err(|e| (FS_ACCESS_DENIED, e))
}
fn internal<T>(r: std::io::Result<T>, operation: &str) -> Result<T, (i32, String)> {
    r.map_err(|e| (INTERNAL_ERROR, format!("{operation} failed: {e}")))
}

fn read(v: &serde_json::Value, w: &WorkspaceRoots) -> Result<serde_json::Value, (i32, String)> {
    let p: PathParams = params(v, "fs/read")?;
    let path = access(w.existing_path(&p.path))?;
    let m = internal(std::fs::metadata(&path), "read metadata")?;
    if !m.is_file() || m.len() > MAX_READ_BYTES {
        return Err((
            FS_ACCESS_DENIED,
            "file is not readable or exceeds 100MB".into(),
        ));
    }
    let content = internal(std::fs::read_to_string(path), "read")?;
    Ok(serde_json::json!({"size":content.len(),"content":content}))
}
fn write(v: &serde_json::Value, w: &WorkspaceRoots) -> Result<serde_json::Value, (i32, String)> {
    let p: WriteParams = params(v, "fs/write")?;
    let path = access(w.write_path(&p.path))?;
    enforce_write_policy(&path, &p.deny_exact, p.write_roots.as_deref())?;
    internal(std::fs::write(path, p.content.as_bytes()), "write")?;
    Ok(serde_json::json!({"bytes_written":p.content.len()}))
}

fn apply_patch(
    v: &serde_json::Value,
    workspace: &WorkspaceRoots,
) -> Result<serde_json::Value, (i32, String)> {
    let request: ApplyPatchParams = params(v, "fs/applyPatch")?;
    if request.patch.is_empty() {
        return Err((INVALID_PARAMS, "patch must not be empty".into()));
    }
    let working_dir = access(workspace.execution_dir(request.working_dir.as_deref()))?;
    let targets = patch_targets(&request.patch)?;
    if targets.is_empty() {
        return Err((
            INVALID_PARAMS,
            "patch contains no recognized file targets".into(),
        ));
    }
    for target in &targets {
        // `write_path` resolves existing symlinks and canonicalizes the parent
        // of new paths. This rejects absolute/traversal targets and symlink
        // escapes before the patch process receives any input.
        let resolved = access(workspace.prospective_write_path(&working_dir.join(target)))?;
        enforce_write_policy(
            &resolved,
            &request.deny_exact,
            request.write_roots.as_deref(),
        )?;
    }

    if request.patch.trim_start().starts_with("*** Begin Patch") {
        let patch = corpus::tools::tools::structured_patch::parse_structured_patch(&request.patch)
            .map_err(|error| (INVALID_PARAMS, format!("invalid structured patch: {error}")))?;
        let result =
            corpus::tools::tools::structured_patch::execute_structured_patch(&patch, &working_dir);
        let exit_code = i32::from(!result.failed.is_empty());
        return Ok(serde_json::json!({
            "exit_code": exit_code,
            "stdout": serde_json::to_string_pretty(&result).unwrap_or_default(),
            "stderr": "",
            "targets": targets,
        }));
    }

    // Unified diffs use the standard patch utility only after every old/new
    // target has crossed the canonical path and profile checks above. No shell
    // is involved and the payload is sent over a private stdin pipe.
    let mut child = std::process::Command::new("patch")
        .args(["-p1", "--batch", "--forward"])
        .current_dir(&working_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            (
                INTERNAL_ERROR,
                format!("start unified patch adapter failed: {error}"),
            )
        })?;
    child
        .stdin
        .take()
        .ok_or_else(|| (INTERNAL_ERROR, "unified patch stdin unavailable".into()))?
        .write_all(request.patch.as_bytes())
        .map_err(|error| (INTERNAL_ERROR, format!("write patch input failed: {error}")))?;
    let output = child.wait_with_output().map_err(|error| {
        (
            INTERNAL_ERROR,
            format!("wait for unified patch adapter failed: {error}"),
        )
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(serde_json::json!({
        "exit_code": output.status.code().unwrap_or(-1),
        "stdout": stdout,
        "stderr": stderr,
        "targets": targets,
    }))
}

fn enforce_write_policy(
    resolved: &Path,
    deny_exact: &[PathBuf],
    write_roots: Option<&[PathBuf]>,
) -> Result<(), (i32, String)> {
    if deny_exact.iter().any(|deny| {
        deny.canonicalize()
            .is_ok_and(|deny| resolved == deny || resolved.starts_with(deny))
    }) {
        return Err((
            FS_ACCESS_DENIED,
            format!(
                "write target is denied by sandbox policy: {}",
                resolved.display()
            ),
        ));
    }
    if let Some(write_roots) = write_roots {
        let allowed = write_roots.iter().any(|root| {
            root.canonicalize()
                .is_ok_and(|root| resolved.starts_with(root))
        });
        if !allowed {
            return Err((
                FS_ACCESS_DENIED,
                format!(
                    "write target is outside sandbox writable roots: {}",
                    resolved.display()
                ),
            ));
        }
    }
    Ok(())
}

fn patch_targets(patch: &str) -> Result<Vec<PathBuf>, (i32, String)> {
    let mut targets = Vec::new();
    for line in patch.lines() {
        let candidate = [
            "*** Add File: ",
            "*** Delete File: ",
            "*** Update File: ",
            "*** Move to: ",
            "Add File: ",
            "Delete File: ",
            "Update File: ",
            "Append File: ",
            "Move to: ",
            "+++ ",
            "--- ",
        ]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix));
        let Some(candidate) = candidate else {
            continue;
        };
        let candidate = candidate.split('\t').next().unwrap_or("").trim();
        if candidate == "/dev/null" || candidate.is_empty() {
            continue;
        }
        let candidate = candidate
            .strip_prefix("a/")
            .or_else(|| candidate.strip_prefix("b/"))
            .unwrap_or(candidate);
        let path = Path::new(candidate);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err((
                FS_ACCESS_DENIED,
                format!("patch target is outside the workspace: {candidate}"),
            ));
        }
        let path = path.to_path_buf();
        if !targets.contains(&path) {
            targets.push(path);
        }
    }
    Ok(targets)
}
fn list(v: &serde_json::Value, w: &WorkspaceRoots) -> Result<serde_json::Value, (i32, String)> {
    let p: PathParams = params(v, "fs/list")?;
    let path = access(w.existing_path(&p.path))?;
    let mut entries = Vec::new();
    for e in internal(std::fs::read_dir(path), "list")? {
        let e = internal(e, "list entry")?;
        let t = internal(e.file_type(), "entry metadata")?;
        entries.push(serde_json::json!({"name":e.file_name().to_string_lossy(),"kind":if t.is_dir(){"dir"}else if t.is_file(){"file"}else{"other"}}));
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(serde_json::json!({"entries":entries}))
}
fn metadata(v: &serde_json::Value, w: &WorkspaceRoots) -> Result<serde_json::Value, (i32, String)> {
    let p: PathParams = params(v, "fs/metadata")?;
    let path = access(w.existing_path(&p.path))?;
    let m = internal(std::fs::metadata(path), "metadata")?;
    let modified = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis().to_string());
    Ok(
        serde_json::json!({"size":m.len(),"modified":modified,"is_file":m.is_file(),"is_dir":m.is_dir()}),
    )
}
fn walk(v: &serde_json::Value, w: &WorkspaceRoots) -> Result<serde_json::Value, (i32, String)> {
    let p: WalkParams = params(v, "fs/walk")?;
    let root = access(w.existing_path(&p.path))?;
    let mut out = Vec::new();
    walk_dir(&root, &root, 0, p.max_depth, &mut out)?;
    Ok(serde_json::json!({"files":out}))
}
fn walk_dir(
    root: &Path,
    dir: &Path,
    depth: usize,
    max: usize,
    out: &mut Vec<serde_json::Value>,
) -> Result<(), (i32, String)> {
    if depth > max {
        return Ok(());
    }
    for e in internal(std::fs::read_dir(dir), "walk")? {
        let e = internal(e, "walk entry")?;
        let t = internal(e.file_type(), "walk metadata")?;
        if t.is_symlink() {
            continue;
        }
        let path = e.path();
        out.push(serde_json::json!({"path":path.strip_prefix(root).unwrap_or(&path),"kind":if t.is_dir(){"dir"}else{"file"}}));
        if t.is_dir() && depth < max {
            walk_dir(root, &path, depth + 1, max, out)?;
        }
    }
    Ok(())
}
fn remove(v: &serde_json::Value, w: &WorkspaceRoots) -> Result<serde_json::Value, (i32, String)> {
    let p: PathParams = params(v, "fs/remove")?;
    let path = access(w.existing_path(&p.path))?;
    if w.roots.contains(&path) {
        return Err((FS_ACCESS_DENIED, "workspace root cannot be removed".into()));
    }
    let m = internal(std::fs::metadata(&path), "remove metadata")?;
    if m.is_dir() {
        internal(std::fs::remove_dir_all(path), "remove directory")?
    } else {
        internal(std::fs::remove_file(path), "remove file")?
    }
    Ok(serde_json::json!({}))
}
fn copy(v: &serde_json::Value, w: &WorkspaceRoots) -> Result<serde_json::Value, (i32, String)> {
    let p: CopyParams = params(v, "fs/copy")?;
    let src = access(w.existing_path(&p.source))?;
    let dst = access(w.write_path(&p.dest))?;
    if src.is_dir() && dst.starts_with(&src) {
        return Err((
            FS_ACCESS_DENIED,
            "copy destination cannot be inside source directory".into(),
        ));
    }
    copy_entry(&src, &dst)?;
    Ok(serde_json::json!({}))
}
fn copy_entry(src: &Path, dst: &Path) -> Result<(), (i32, String)> {
    let m = internal(std::fs::symlink_metadata(src), "copy metadata")?;
    if m.file_type().is_symlink() {
        return Err((FS_ACCESS_DENIED, "copying symlinks is denied".into()));
    }
    if m.is_dir() {
        internal(std::fs::create_dir_all(dst), "copy directory")?;
        for e in internal(std::fs::read_dir(src), "copy directory")? {
            let e = internal(e, "copy entry")?;
            copy_entry(&e.path(), &dst.join(e.file_name()))?;
        }
    } else {
        internal(std::fs::copy(src, dst), "copy file")?;
    }
    Ok(())
}
fn open(
    v: &serde_json::Value,
    w: &WorkspaceRoots,
    f: &FileManager,
) -> Result<serde_json::Value, (i32, String)> {
    let p: OpenParams = params(v, "fs/open")?;
    let mut options = OpenOptions::new();
    let path = match p.mode.as_str() {
        "read" => {
            options.read(true);
            let path = access(w.existing_path(&p.path))?;
            let metadata = internal(std::fs::metadata(&path), "open metadata")?;
            if !metadata.is_file() || metadata.len() > MAX_READ_BYTES {
                return Err((
                    FS_ACCESS_DENIED,
                    "file is not readable or exceeds 100MB".into(),
                ));
            }
            path
        }
        "write" => {
            options.write(true).create(true).truncate(true);
            access(w.write_path(&p.path))?
        }
        "append" => {
            options.append(true).create(true);
            access(w.write_path(&p.path))?
        }
        _ => return Err((INVALID_PARAMS, "mode must be read, write, or append".into())),
    };
    let file = internal(options.open(path), "open")?;
    let mut state = f
        .inner
        .lock()
        .map_err(|_| (INTERNAL_ERROR, "file handle table poisoned".into()))?;
    if state.files.len() >= MAX_FILE_HANDLES {
        return Err((
            BUFFER_OVERFLOW,
            "maximum 128 open file handles reached".into(),
        ));
    }
    let handle = format!("file-{}", state.next);
    state.next = state.next.wrapping_add(1);
    state.files.insert(handle.clone(), OpenFile { file });
    Ok(serde_json::json!({"handle":handle}))
}
fn close(v: &serde_json::Value, f: &FileManager) -> Result<serde_json::Value, (i32, String)> {
    let p: HandleParams = params(v, "fs/close")?;
    let mut state = f
        .inner
        .lock()
        .map_err(|_| (INTERNAL_ERROR, "file handle table poisoned".into()))?;
    if state.files.remove(&p.handle).is_none() {
        return Err((FS_HANDLE_NOT_FOUND, "file handle not found".into()));
    }
    Ok(serde_json::json!({}))
}
fn read_chunk(v: &serde_json::Value, f: &FileManager) -> Result<serde_json::Value, (i32, String)> {
    let p: ReadChunkParams = params(v, "fs/readChunk")?;
    if p.size > MAX_CHUNK_BYTES {
        return Err((BUFFER_OVERFLOW, "read chunk exceeds 1MB".into()));
    }
    let mut state = f
        .inner
        .lock()
        .map_err(|_| (INTERNAL_ERROR, "file handle table poisoned".into()))?;
    let open = state
        .files
        .get_mut(&p.handle)
        .ok_or((FS_HANDLE_NOT_FOUND, "file handle not found".into()))?;
    internal(open.file.seek(SeekFrom::Start(p.offset)), "seek")?;
    let mut data = vec![0; p.size];
    let n = internal(open.file.read(&mut data), "read chunk")?;
    data.truncate(n);
    let eof = p.offset.saturating_add(n as u64)
        >= internal(open.file.metadata(), "read chunk metadata")?.len();
    Ok(serde_json::json!({"data":String::from_utf8_lossy(&data),"eof":eof}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_filesystem_methods_preserve_workspace_confinement() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(root.join("nested/deeper")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("allowed.txt"), "abcdef").unwrap();
        std::fs::write(root.join("nested/deeper/value"), "v").unwrap();
        std::fs::write(outside.join("secret"), "nope").unwrap();
        let roots = WorkspaceRoots::new(vec![root.clone()]).unwrap();
        let files = FileManager::default();

        let listed = list(&serde_json::json!({"path": root}), &roots).unwrap();
        assert!(listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "allowed.txt"));
        let meta = metadata(
            &serde_json::json!({"path": root.join("allowed.txt")}),
            &roots,
        )
        .unwrap();
        assert_eq!(meta["size"], 6);
        let walked = walk(&serde_json::json!({"path": root, "max_depth": 0}), &roots).unwrap();
        assert!(!walked["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"].as_str().unwrap().contains("deeper/value")));

        copy(
            &serde_json::json!({"source": root.join("allowed.txt"), "dest": root.join("copy.txt")}),
            &roots,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("copy.txt")).unwrap(),
            "abcdef"
        );
        remove(&serde_json::json!({"path": root.join("copy.txt")}), &roots).unwrap();
        assert!(!root.join("copy.txt").exists());

        for method in ["fs/list", "fs/metadata", "fs/walk", "fs/remove", "fs/open"] {
            let value = match method {
                "fs/walk" => serde_json::json!({"path": outside, "max_depth": 1}),
                "fs/open" => serde_json::json!({"path": outside.join("secret"), "mode":"read"}),
                _ => serde_json::json!({"path": outside.join("secret")}),
            };
            let response = handle_fs(method, &value, &roots, &files).unwrap();
            assert!(
                matches!(response.result, ResponseResult::Err { ref error } if error.code == FS_ACCESS_DENIED)
            );
        }
        assert!(matches!(
            copy(
                &serde_json::json!({"source": outside.join("secret"), "dest": root.join("x")}),
                &roots
            ),
            Err((FS_ACCESS_DENIED, _))
        ));
    }

    #[test]
    fn chunk_handles_are_bounded_closeable_and_offset_based() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        std::fs::write(&path, "abcdef").unwrap();
        let roots = WorkspaceRoots::new(vec![temp.path().to_path_buf()]).unwrap();
        let files = FileManager::default();
        let mut handles = Vec::new();
        for _ in 0..MAX_FILE_HANDLES {
            handles.push(
                open(
                    &serde_json::json!({"path": path, "mode":"read"}),
                    &roots,
                    &files,
                )
                .unwrap()["handle"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
        assert!(matches!(
            open(
                &serde_json::json!({"path": path, "mode":"read"}),
                &roots,
                &files
            ),
            Err((BUFFER_OVERFLOW, _))
        ));
        let chunk = read_chunk(
            &serde_json::json!({"handle": handles[0], "offset":2, "size":3}),
            &files,
        )
        .unwrap();
        assert_eq!(chunk["data"], "cde");
        assert_eq!(chunk["eof"], false);
        close(&serde_json::json!({"handle": handles[0]}), &files).unwrap();
        assert!(matches!(
            read_chunk(
                &serde_json::json!({"handle": handles[0], "offset":0, "size":1}),
                &files
            ),
            Err((FS_HANDLE_NOT_FOUND, _))
        ));
        assert!(open(
            &serde_json::json!({"path": path, "mode":"read"}),
            &roots,
            &files
        )
        .is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_denied_for_new_operations() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), "nope").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        let roots = WorkspaceRoots::new(vec![root.clone()]).unwrap();
        let files = FileManager::default();
        for method in ["fs/list", "fs/metadata", "fs/open"] {
            let params = if method == "fs/open" {
                serde_json::json!({"path":root.join("escape/secret"),"mode":"read"})
            } else {
                serde_json::json!({"path":root.join("escape/secret")})
            };
            let response = handle_fs(method, &params, &roots, &files).unwrap();
            assert!(
                matches!(response.result, ResponseResult::Err { ref error } if error.code == FS_ACCESS_DENIED)
            );
        }
    }

    #[test]
    fn process_working_directory_is_canonical_and_workspace_confined() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let roots = WorkspaceRoots::new(vec![workspace.clone()]).unwrap();
        assert!(roots.execution_dir(Some(&outside)).is_err());
        assert_eq!(
            roots.execution_dir(None).unwrap(),
            workspace.canonicalize().unwrap()
        );
    }

    #[test]
    fn patch_targets_accept_workspace_relative_formats_and_reject_escape() {
        assert_eq!(
            patch_targets("*** Begin Patch\n*** Update File: src/lib.rs\n*** End Patch").unwrap(),
            vec![PathBuf::from("src/lib.rs")]
        );
        assert_eq!(
            patch_targets("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@").unwrap(),
            vec![PathBuf::from("src/lib.rs")]
        );
        assert!(matches!(
            patch_targets("*** Begin Patch\n*** Add File: ../outside\n*** End Patch"),
            Err((FS_ACCESS_DENIED, _))
        ));
        assert!(matches!(
            patch_targets("--- /dev/null\n+++ /tmp/outside\n"),
            Err((FS_ACCESS_DENIED, _))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn patch_target_symlink_escape_is_denied_before_adapter_spawn() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        let roots = WorkspaceRoots::new(vec![root.clone()]).unwrap();
        let result = apply_patch(
            &serde_json::json!({
                "working_dir": root,
                "patch": "*** Begin Patch\n*** Add File: escape/file\n+no\n*** End Patch"
            }),
            &roots,
        );
        assert!(matches!(result, Err((FS_ACCESS_DENIED, _))));
        assert!(!outside.join("file").exists());
    }

    #[test]
    fn patch_target_under_profile_deny_is_rejected_before_adapter_spawn() {
        let temp = tempfile::tempdir().unwrap();
        let denied = temp.path().join("credentials");
        std::fs::create_dir_all(&denied).unwrap();
        let roots = WorkspaceRoots::new(vec![temp.path().to_path_buf()]).unwrap();
        let result = apply_patch(
            &serde_json::json!({
                "working_dir": temp.path(),
                "deny_exact": [denied],
                "patch": "*** Begin Patch\n*** Add File: credentials/token\n+no\n*** End Patch"
            }),
            &roots,
        );
        assert!(matches!(result, Err((FS_ACCESS_DENIED, _))));
    }

    #[test]
    fn direct_write_obeys_profile_writable_roots_and_deny_paths() {
        let temp = tempfile::tempdir().unwrap();
        let allowed = temp.path().join("allowed");
        let denied = allowed.join("credentials");
        let outside_write_root = temp.path().join("other");
        std::fs::create_dir_all(&denied).unwrap();
        std::fs::create_dir_all(&outside_write_root).unwrap();
        let roots = WorkspaceRoots::new(vec![temp.path().to_path_buf()]).unwrap();
        assert!(write(
            &serde_json::json!({
                "path": allowed.join("ok"),
                "content": "ok",
                "write_roots": [&allowed],
                "deny_exact": [&denied],
            }),
            &roots,
        )
        .is_ok());
        assert!(matches!(
            write(
                &serde_json::json!({
                    "path": denied.join("token"),
                    "content": "no",
                    "write_roots": [&allowed],
                    "deny_exact": [&denied],
                }),
                &roots,
            ),
            Err((FS_ACCESS_DENIED, _))
        ));
        assert!(matches!(
            write(
                &serde_json::json!({
                    "path": outside_write_root.join("no"),
                    "content": "no",
                    "write_roots": [&allowed],
                }),
                &roots,
            ),
            Err((FS_ACCESS_DENIED, _))
        ));
    }

    #[test]
    fn structured_patch_is_applied_in_process_through_confined_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let roots = WorkspaceRoots::new(vec![temp.path().to_path_buf()]).unwrap();
        let result = apply_patch(
            &serde_json::json!({
                "working_dir": temp.path(),
                "write_roots": [temp.path()],
                "patch": "*** Begin Patch\nAdd File: created.txt\n>>>\nhello\n>>>\n*** End Patch"
            }),
            &roots,
        )
        .unwrap();
        assert_eq!(result["exit_code"], 0);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("created.txt")).unwrap(),
            "hello"
        );
    }
}
