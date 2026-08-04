//! Production host adapters for daemon startup and readiness negotiation.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use fabric::protocol::client::{
    ClientCapabilities, ClientEvent, ClientMessage, ClientRequest, InitializeParams,
    CLIENT_PROTOCOL_VERSION,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::application::daemon_lifecycle::{
    resolve_install_mode, DaemonActivation, DaemonInstallMode, DaemonLifecycleBackend,
    DaemonLifecycleError, DaemonReadiness, InstallModeFacts, StartupLease, StartupLockPort,
};

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(20);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(700);
const MAX_DIAGNOSTIC_BYTES: usize = 4096;

struct FileLease {
    _file: File,
}

impl StartupLease for FileLease {}

pub struct FileStartupLock {
    path: PathBuf,
}

impl FileStartupLock {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[async_trait::async_trait]
impl StartupLockPort for FileStartupLock {
    async fn acquire(
        &self,
        timeout: Duration,
    ) -> Result<Box<dyn StartupLease>, DaemonLifecycleError> {
        let deadline = Instant::now() + timeout;
        loop {
            match try_file_lock(&self.path).map_err(|error| {
                DaemonLifecycleError::Lock(format!("{}: {error}", self.path.display()))
            })? {
                Some(file) => return Ok(Box::new(FileLease { _file: file })),
                None if Instant::now() >= deadline => {
                    return Err(DaemonLifecycleError::LockTimeout {
                        timeout_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
                    })
                }
                None => tokio::time::sleep(LOCK_POLL_INTERVAL).await,
            }
        }
    }
}

/// Held for the complete daemon process lifetime. Kernel advisory locks are
/// released automatically on crash, so stale lock files are harmless.
pub struct DaemonAuthorityLease {
    _file: File,
}

pub fn acquire_daemon_authority(path: &Path) -> anyhow::Result<DaemonAuthorityLease> {
    match try_file_lock(path)? {
        Some(file) => Ok(DaemonAuthorityLease { _file: file }),
        None => anyhow::bail!(
            "another daemon authority already holds the runtime lock '{}'",
            path.display()
        ),
    }
}

fn try_file_lock(path: &Path) -> io::Result<Option<File>> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "lock path is not a regular file",
        ));
    }
    let expected_uid = nix::unistd::Uid::effective().as_raw();
    if metadata.uid() != expected_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "lock file is owned by uid {}, expected {}",
                metadata.uid(),
                expected_uid
            ),
        ));
    }

    // SAFETY: flock borrows a valid open descriptor and does not take ownership.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(Some(file));
    }
    let error = io::Error::last_os_error();
    if error
        .raw_os_error()
        .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN)
    {
        Ok(None)
    } else {
        Err(error)
    }
}

pub struct ProcessDaemonLifecycleBackend {
    executable: PathBuf,
}

impl ProcessDaemonLifecycleBackend {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }
}

#[async_trait::async_trait]
impl DaemonLifecycleBackend for ProcessDaemonLifecycleBackend {
    async fn probe(&self, socket: &Path) -> Result<DaemonReadiness, DaemonLifecycleError> {
        probe_daemon(socket).await
    }

    async fn recover_stale_socket(&self, socket: &Path) -> Result<(), DaemonLifecycleError> {
        let metadata = match std::fs::symlink_metadata(socket) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(DaemonLifecycleError::StaleSocketRecovery(bounded(
                    error.to_string().as_bytes(),
                )))
            }
        };
        let expected_uid = nix::unistd::Uid::effective().as_raw();
        if !metadata.file_type().is_socket() || metadata.uid() != expected_uid {
            return Err(DaemonLifecycleError::StaleSocketRecovery(format!(
                "refusing to remove non-owned socket path '{}'",
                socket.display()
            )));
        }
        tokio::fs::remove_file(socket).await.map_err(|error| {
            DaemonLifecycleError::StaleSocketRecovery(bounded(error.to_string().as_bytes()))
        })
    }

    async fn activate(
        &self,
        mode: DaemonInstallMode,
        socket: &Path,
    ) -> Result<DaemonActivation, DaemonLifecycleError> {
        match mode {
            DaemonInstallMode::SystemInstall | DaemonInstallMode::UserLocal => {
                activate_user_socket().await?;
                Ok(DaemonActivation::ActivatedService)
            }
            DaemonInstallMode::DevForeground => {
                spawn_foreground(&self.executable, socket).await?;
                Ok(DaemonActivation::SpawnedForeground)
            }
        }
    }

    async fn diagnose(&self, socket: &Path) -> String {
        let mut command = Command::new("systemctl");
        command.args([
            "--user",
            "show",
            "aletheon.service",
            "--property=LoadState,ActiveState,SubState,Result,NRestarts",
            "--no-pager",
        ]);
        let output = command_output(command).await;
        let service = match output {
            Ok(output) => {
                let bytes = if output.status.success() {
                    output.stdout
                } else {
                    output.stderr
                };
                bounded(&bytes)
            }
            Err(error) => bounded(error.to_string().as_bytes()),
        };
        format!(
            "socket={} exists={}; service={}",
            socket.display(),
            socket.exists(),
            service.replace('\n', ", ")
        )
    }
}

pub async fn detect_install_mode() -> DaemonInstallMode {
    let mut fragment_command = Command::new("systemctl");
    fragment_command.args([
        "--user",
        "show",
        "aletheon.socket",
        "--property=FragmentPath",
        "--value",
    ]);
    let fragment = command_output(fragment_command)
        .await
        .ok()
        .filter(|output| output.status.success())
        .map(|output| bounded(&output.stdout))
        .unwrap_or_default();
    let mut exec_command = Command::new("systemctl");
    exec_command.args([
        "--user",
        "show",
        "aletheon.service",
        "--property=ExecStart",
        "--value",
    ]);
    let exec_start = command_output(exec_command)
        .await
        .ok()
        .filter(|output| output.status.success())
        .map(|output| bounded(&output.stdout))
        .unwrap_or_default();
    resolve_mode_from_systemd(
        &fragment,
        &exec_start,
        Path::new("/usr/bin/aletheon").is_file(),
    )
}

fn resolve_mode_from_systemd(
    fragment: &str,
    exec_start: &str,
    system_binary_available: bool,
) -> DaemonInstallMode {
    let fragment = Path::new(fragment.trim());
    let service_available = !fragment.as_os_str().is_empty();
    let executable_known = !exec_start.trim().is_empty();
    let system_service = exec_start.contains("/usr/bin/aletheon");
    resolve_install_mode(InstallModeFacts {
        system_install_available: service_available
            && executable_known
            && system_service
            && system_binary_available,
        user_local_install_available: service_available && executable_known && !system_service,
    })
}

async fn activate_user_socket() -> Result<(), DaemonLifecycleError> {
    let mut command = Command::new("systemctl");
    command.args(["--user", "start", "aletheon.socket"]);
    let output = command_output(command)
        .await
        .map_err(|error| DaemonLifecycleError::Activation(bounded(error.to_string().as_bytes())))?;
    if !output.status.success() {
        return Err(DaemonLifecycleError::Activation(bounded(&output.stderr)));
    }
    Ok(())
}

async fn spawn_foreground(executable: &Path, socket: &Path) -> Result<(), DaemonLifecycleError> {
    let mut command = Command::new(executable);
    command
        .arg("daemon")
        .arg("--socket")
        .arg(socket)
        .env_remove("LISTEN_PID")
        .env_remove("LISTEN_FDS")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = tokio::task::spawn_blocking(move || command.spawn())
        .await
        .map_err(|error| DaemonLifecycleError::Activation(bounded(error.to_string().as_bytes())))?
        .map_err(|error| DaemonLifecycleError::Activation(bounded(error.to_string().as_bytes())))?;
    std::thread::spawn(move || match child.wait() {
        Ok(status) if !status.success() => {
            tracing::warn!(%status, "development daemon exited unsuccessfully")
        }
        Err(error) => tracing::warn!(%error, "failed to reap development daemon"),
        _ => {}
    });
    Ok(())
}

async fn command_output(mut command: Command) -> io::Result<Output> {
    tokio::task::spawn_blocking(move || command.output())
        .await
        .map_err(io::Error::other)?
}

async fn probe_daemon(socket: &Path) -> Result<DaemonReadiness, DaemonLifecycleError> {
    if !socket.exists() {
        return Ok(DaemonReadiness::Absent);
    }
    let stream = match tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(socket)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error))
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            return Ok(DaemonReadiness::StaleSocket {
                detail: bounded(error.to_string().as_bytes()),
            })
        }
        Ok(Err(error)) => {
            return Ok(DaemonReadiness::Unready {
                detail: bounded(error.to_string().as_bytes()),
            })
        }
        Err(_) => {
            return Ok(DaemonReadiness::Unready {
                detail: "socket connection timed out".into(),
            })
        }
    };

    let (reader, mut writer) = stream.into_split();
    let request = ClientRequest::Initialize(InitializeParams {
        client_version: env!("CARGO_PKG_VERSION").into(),
        protocol_versions: vec![CLIENT_PROTOCOL_VERSION],
        capabilities: ClientCapabilities {
            item_events: false,
            cursors: false,
            memory_gateway_v1: false,
            memory_maintenance_v1: false,
            memory_admin_v1: false,
        },
    })
    .to_json_rpc(1)
    .map_err(|error| DaemonLifecycleError::Probe(error.to_string()))?;
    let mut payload = serde_json::to_vec(&request)
        .map_err(|error| DaemonLifecycleError::Probe(error.to_string()))?;
    payload.push(b'\n');
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, writer.write_all(&payload)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return Ok(DaemonReadiness::Unready {
                detail: bounded(error.to_string().as_bytes()),
            })
        }
        Err(_) => {
            return Ok(DaemonReadiness::Unready {
                detail: "initialize write timed out".into(),
            })
        }
    }

    let mut reader = BufReader::new(reader);
    let mut response = String::new();
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.read_line(&mut response)).await {
        Ok(Ok(0)) => {
            return Ok(DaemonReadiness::Unready {
                detail: "daemon closed during initialize".into(),
            })
        }
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            return Ok(DaemonReadiness::Unready {
                detail: bounded(error.to_string().as_bytes()),
            })
        }
        Err(_) => {
            return Ok(DaemonReadiness::Unready {
                detail: "initialize response timed out".into(),
            })
        }
    }

    let response: serde_json::Value = serde_json::from_str(response.trim()).map_err(|error| {
        DaemonLifecycleError::Probe(format!("invalid initialize JSON: {error}"))
    })?;
    if let Some(error) = response.get("error") {
        return Err(DaemonLifecycleError::Probe(format!(
            "initialize rejected: {}",
            bounded(error.to_string().as_bytes())
        )));
    }
    let message: ClientMessage<ClientEvent> = serde_json::from_value(response["result"].clone())
        .map_err(|error| {
            DaemonLifecycleError::Probe(format!("invalid initialize response: {error}"))
        })?;
    match message.payload {
        ClientEvent::InitializeResponse(initialized) => Ok(DaemonReadiness::Ready {
            protocol_version: initialized.protocol_version,
            runtime_version: initialized.runtime_version,
        }),
        event => Err(DaemonLifecycleError::Probe(format!(
            "unexpected initialize event: {event:?}"
        ))),
    }
}

fn bounded(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_DIAGNOSTIC_BYTES)])
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_mode_uses_typed_unit_provenance() {
        assert_eq!(
            resolve_mode_from_systemd(
                "/home/a/.config/systemd/user/aletheon.socket",
                "{ path=/usr/bin/aletheon ; argv[]=/usr/bin/aletheon daemon ; }",
                true,
            ),
            DaemonInstallMode::SystemInstall
        );
        assert_eq!(
            resolve_mode_from_systemd(
                "/home/a/.config/systemd/user/aletheon.socket",
                "{ path=/home/a/.local/bin/aletheon ; argv[]=/home/a/.local/bin/aletheon daemon ; }",
                false,
            ),
            DaemonInstallMode::UserLocal
        );
        assert_eq!(
            resolve_mode_from_systemd("", "", false),
            DaemonInstallMode::DevForeground
        );
        assert_eq!(
            resolve_mode_from_systemd("/home/a/.config/systemd/user/aletheon.socket", "", false,),
            DaemonInstallMode::DevForeground
        );
    }

    #[tokio::test]
    async fn file_lock_serializes_and_recovers_after_drop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("startup.lock");
        let first = FileStartupLock::new(path.clone());
        let second = FileStartupLock::new(path);
        let lease = first.acquire(Duration::from_secs(1)).await.unwrap();
        assert!(matches!(
            second.acquire(Duration::from_millis(20)).await,
            Err(DaemonLifecycleError::LockTimeout { .. })
        ));
        drop(lease);
        second.acquire(Duration::from_secs(1)).await.unwrap();
    }

    #[test]
    fn daemon_authority_lock_rejects_a_second_writer_and_recovers_after_drop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("authority.lock");
        let first = acquire_daemon_authority(&path).unwrap();
        assert!(acquire_daemon_authority(&path).is_err());
        drop(first);
        acquire_daemon_authority(&path).unwrap();
    }

    #[tokio::test]
    async fn readiness_probe_negotiates_typed_runtime_facts() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("daemon.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut request = String::new();
            reader.read_line(&mut request).await.unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&request).unwrap()["method"],
                "initialize"
            );
            let event =
                ClientEvent::InitializeResponse(fabric::protocol::client::InitializedResult {
                    protocol_version: CLIENT_PROTOCOL_VERSION,
                    server_capabilities: ClientCapabilities {
                        item_events: false,
                        cursors: false,
                        memory_gateway_v1: false,
                        memory_maintenance_v1: false,
                        memory_admin_v1: false,
                    },
                    connection_id: fabric::ConnectionId::new(),
                    principal_id: fabric::PrincipalId::local_uid(1000),
                    os_principal: fabric::LocalOsPrincipal {
                        uid: 1000,
                        gid: 1000,
                    },
                    runtime_version: "test-runtime".into(),
                });
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": ClientMessage::v1(event),
            });
            writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        });

        assert_eq!(
            probe_daemon(&socket).await.unwrap(),
            DaemonReadiness::Ready {
                protocol_version: CLIENT_PROTOCOL_VERSION,
                runtime_version: "test-runtime".into(),
            }
        );
        server.await.unwrap();
    }
}
