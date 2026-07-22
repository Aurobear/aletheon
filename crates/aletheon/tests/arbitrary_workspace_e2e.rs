use std::path::{Path, PathBuf};
use std::sync::Arc;

use executive::host::core_rpc::{CorePeerPolicy, CoreRpcServer};
use executive::application::inference_port::{CoreInferenceRequest, InferenceError, InferencePort};
use executive::{
    ContentBlock, LlmResponse, LlmStream, LocalOsPrincipal, StopReason, StreamChunk, Usage,
};
use futures::stream;
use tempfile::TempDir;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct FakeInference;

#[async_trait::async_trait]
impl InferencePort for FakeInference {
    async fn complete(
        &self,
        _request: CoreInferenceRequest,
    ) -> Result<LlmResponse, InferenceError> {
        Ok(LlmResponse {
            content: vec![ContentBlock::Text { text: "ok".into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
        })
    }

    async fn stream(&self, _request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        Ok(Box::pin(stream::iter([
            Ok(StreamChunk::TextDelta { text: "ok".into() }),
            Ok(StreamChunk::Done {
                stop_reason: StopReason::EndTurn,
            }),
        ])))
    }
}

struct CoreHarness {
    socket: PathBuf,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
    peers: mpsc::Receiver<LocalOsPrincipal>,
}

impl CoreHarness {
    async fn start(root: &Path) -> Self {
        let socket = root.join("core.sock");
        let uid = effective_uid();
        let gid = effective_gid();
        let (peer_tx, peers) = mpsc::channel(16);
        let server = CoreRpcServer::bind_with_limit(
            &socket,
            Arc::new(FakeInference),
            CorePeerPolicy::new(uid, gid, [uid]),
            1024 * 1024,
            Some(peer_tx),
        )
        .await
        .unwrap();
        let cancel = CancellationToken::new();
        let server_cancel = cancel.clone();
        let task = tokio::spawn(async move { server.run(server_cancel).await });
        Self {
            socket,
            cancel,
            task,
            peers,
        }
    }

    async fn next_peer(&mut self) -> LocalOsPrincipal {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.peers.recv())
            .await
            .expect("core peer observation timed out")
            .expect("core peer observer closed")
    }

    async fn shutdown(self) {
        self.cancel.cancel();
        self.task.await.unwrap().unwrap();
    }
}

struct IsolatedUser {
    _root: TempDir,
    home: PathBuf,
    runtime: PathBuf,
    state: PathBuf,
    cache: PathBuf,
}

impl IsolatedUser {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let runtime = root.path().join("runtime");
        let state = root.path().join("state");
        let cache = root.path().join("cache");
        for path in [&home, &runtime, &state, &cache] {
            std::fs::create_dir(path).unwrap();
        }
        Self {
            _root: root,
            home,
            runtime,
            state,
            cache,
        }
    }

    fn state_root(&self) -> PathBuf {
        self.state.join("aletheon")
    }

    async fn exec(&self, core_socket: &Path, cwd: &Path, sandbox: &str) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_aletheon"))
            .args([
                "exec",
                "--prompt",
                "return a minimal response",
                "--model",
                "fake/test",
                "--max-turns",
                "1",
                "--sandbox",
                sandbox,
            ])
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("ALETHEON_CORE_SOCKET", core_socket)
            .env_remove("ALETHEON_CONFIG")
            .output()
            .await
            .unwrap()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launches_from_repo_non_repo_and_tmp_like_directories() {
    let fixture = tempfile::tempdir().unwrap();
    let repo = fixture.path().join("repo");
    let plain = fixture.path().join("plain");
    let tmp_like = fixture.path().join("tmp-like");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir(&plain).unwrap();
    std::fs::create_dir(&tmp_like).unwrap();

    let user = IsolatedUser::new();
    let mut core = CoreHarness::start(fixture.path()).await;
    for cwd in [&repo, &plain, &tmp_like] {
        let output = user.exec(&core.socket, cwd, "auto").await;
        assert!(
            output.status.success(),
            "{} failed\nstdout: {}\nstderr: {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let peer = core.next_peer().await;
        assert_eq!((peer.uid, peer.gid), (effective_uid(), effective_gid()));
    }
    assert_owned_by_current_user(&user.state_root());
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn danger_full_access_never_changes_os_identity() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let user = IsolatedUser::new();
    let mut core = CoreHarness::start(fixture.path()).await;

    let output = user
        .exec(&core.socket, &workspace, "danger-full-access")
        .await;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let peer = core.next_peer().await;
    assert_eq!((peer.uid, peer.gid), (effective_uid(), effective_gid()));
    assert_owned_by_current_user(&user.state_root());
    core.shutdown().await;
}

fn assert_owned_by_current_user(root: &Path) {
    use std::os::unix::fs::MetadataExt;

    let mut pending = vec![root.to_path_buf()];
    let mut observed_file = false;
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path)
            .unwrap_or_else(|error| panic!("missing runtime state {}: {error}", path.display()));
        assert_eq!(
            metadata.uid(),
            effective_uid(),
            "wrong uid: {}",
            path.display()
        );
        assert_eq!(
            metadata.gid(),
            effective_gid(),
            "wrong gid: {}",
            path.display()
        );
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        } else {
            observed_file = true;
        }
    }
    assert!(
        observed_file,
        "exec created no state files under {}",
        root.display()
    );
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not mutate process state.
    unsafe { libc::geteuid() }
}

fn effective_gid() -> u32 {
    // SAFETY: getegid has no preconditions and does not mutate process state.
    unsafe { libc::getegid() }
}
