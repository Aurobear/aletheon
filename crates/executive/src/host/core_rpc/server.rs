use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use futures::StreamExt;
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::application::inference_port::InferencePort;
use fabric::LocalOsPrincipal;

use super::protocol::{
    read_json_line, write_json_line, CoreFrame, CoreRequest, DEFAULT_MAX_FRAME_BYTES,
};

#[derive(Clone, Debug)]
pub struct CorePeerPolicy {
    service_uid: u32,
    group_gid: u32,
    allowed_uids: HashSet<u32>,
}

impl CorePeerPolicy {
    pub fn new(
        service_uid: u32,
        group_gid: u32,
        allowed_uids: impl IntoIterator<Item = u32>,
    ) -> Self {
        Self {
            service_uid,
            group_gid,
            allowed_uids: allowed_uids.into_iter().collect(),
        }
    }

    pub fn authorize(&self, principal: LocalOsPrincipal) -> anyhow::Result<()> {
        self.authorize_with_groups(principal, &[])
    }

    fn authorize_with_groups(
        &self,
        principal: LocalOsPrincipal,
        supplementary_groups: &[u32],
    ) -> anyhow::Result<()> {
        if principal.uid == 0
            || principal.uid == self.service_uid
            || self.allowed_uids.contains(&principal.uid)
            || principal.gid == self.group_gid
            || supplementary_groups.contains(&self.group_gid)
        {
            return Ok(());
        }
        anyhow::bail!("core RPC access denied for uid {}", principal.uid)
    }
}

pub struct CoreRpcServer {
    listener: UnixListener,
    inference: Arc<dyn InferencePort>,
    policy: CorePeerPolicy,
    max_frame_bytes: usize,
    peer_observer: Option<mpsc::Sender<LocalOsPrincipal>>,
}

impl CoreRpcServer {
    pub async fn bind(
        socket_path: &Path,
        inference: Arc<dyn InferencePort>,
        policy: CorePeerPolicy,
    ) -> anyhow::Result<Self> {
        Self::bind_with_limit(
            socket_path,
            inference,
            policy,
            DEFAULT_MAX_FRAME_BYTES,
            None,
        )
        .await
    }

    pub async fn bind_with_limit(
        socket_path: &Path,
        inference: Arc<dyn InferencePort>,
        policy: CorePeerPolicy,
        max_frame_bytes: usize,
        peer_observer: Option<mpsc::Sender<LocalOsPrincipal>>,
    ) -> anyhow::Result<Self> {
        if max_frame_bytes == 0 {
            anyhow::bail!("core RPC frame limit must be positive");
        }
        if socket_path.exists() {
            tokio::fs::remove_file(socket_path).await?;
        }
        let listener = UnixListener::bind(socket_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))?;
        }
        Ok(Self {
            listener,
            inference,
            policy,
            max_frame_bytes,
            peer_observer,
        })
    }

    pub async fn run(self, cancel: CancellationToken) -> anyhow::Result<()> {
        loop {
            let accepted = tokio::select! {
                result = self.listener.accept() => result,
                _ = cancel.cancelled() => return Ok(()),
            };
            let (stream, _) = accepted?;
            let credentials = stream.peer_cred()?;
            let principal = LocalOsPrincipal {
                uid: credentials.uid(),
                gid: credentials.gid(),
            };
            let supplementary = credentials
                .pid()
                .and_then(|pid| supplementary_groups(pid).ok())
                .unwrap_or_default();
            if let Err(error) = self.policy.authorize_with_groups(principal, &supplementary) {
                tracing::warn!(%error, uid = principal.uid, "Rejected core RPC peer");
                continue;
            }
            if let Some(observer) = &self.peer_observer {
                let _ = observer.send(principal).await;
            }
            let inference = self.inference.clone();
            let max_frame_bytes = self.max_frame_bytes;
            tokio::spawn(async move {
                if let Err(error) = handle_connection(stream, inference, max_frame_bytes).await {
                    tracing::warn!(%error, uid = principal.uid, "Core RPC connection failed");
                }
            });
        }
    }
}

fn supplementary_groups(pid: i32) -> anyhow::Result<Vec<u32>> {
    if pid <= 0 {
        anyhow::bail!("invalid core RPC peer pid {pid}");
    }
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))?;
    let groups = status
        .lines()
        .find_map(|line| line.strip_prefix("Groups:"))
        .unwrap_or_default()
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<Vec<u32>, _>>()?;
    Ok(groups)
}

async fn handle_connection(
    stream: UnixStream,
    inference: Arc<dyn InferencePort>,
    max_frame_bytes: usize,
) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut request_ids = HashSet::new();
    loop {
        let request = match read_json_line::<_, CoreRequest>(&mut reader, max_frame_bytes).await {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(()),
            Err(error) => {
                let _ = write_json_line(
                    &mut writer,
                    &CoreFrame::Error {
                        id: 0,
                        message: error.to_string(),
                    },
                    max_frame_bytes,
                )
                .await;
                return Err(error);
            }
        };
        let id = request.id();
        if !request_ids.insert(id) {
            let message = format!("duplicate request id {id}");
            write_json_line(
                &mut writer,
                &CoreFrame::Error {
                    id,
                    message: message.clone(),
                },
                max_frame_bytes,
            )
            .await?;
            anyhow::bail!(message);
        }
        match request {
            CoreRequest::Complete { request, .. } => match inference.complete(request).await {
                Ok(response) => {
                    write_json_line(
                        &mut writer,
                        &CoreFrame::Response { id, response },
                        max_frame_bytes,
                    )
                    .await?;
                }
                Err(error) => {
                    write_json_line(
                        &mut writer,
                        &CoreFrame::Error {
                            id,
                            message: error.to_string(),
                        },
                        max_frame_bytes,
                    )
                    .await?;
                }
            },
            CoreRequest::Stream { request, .. } => match inference.stream(request).await {
                Ok(mut stream) => {
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(chunk) => {
                                write_json_line(
                                    &mut writer,
                                    &CoreFrame::Chunk { id, chunk },
                                    max_frame_bytes,
                                )
                                .await?;
                            }
                            Err(error) => {
                                write_json_line(
                                    &mut writer,
                                    &CoreFrame::Error {
                                        id,
                                        message: error.to_string(),
                                    },
                                    max_frame_bytes,
                                )
                                .await?;
                                return Ok(());
                            }
                        }
                    }
                    write_json_line(&mut writer, &CoreFrame::Completed { id }, max_frame_bytes)
                        .await?;
                }
                Err(error) => {
                    write_json_line(
                        &mut writer,
                        &CoreFrame::Error {
                            id,
                            message: error.to_string(),
                        },
                        max_frame_bytes,
                    )
                    .await?;
                }
            },
        }
    }
}
