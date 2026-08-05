//! Host transaction review CLI projection.
//!
//! The command only submits typed actions and renders authoritative daemon
//! receipts. It does not inspect the workspace or derive settlement locally.

use std::path::PathBuf;

use clap::Subcommand;
use fabric::protocol::client::ClientRpcRequest;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufStream};
use tokio::net::UnixStream;

#[derive(Subcommand)]
pub(crate) enum ReviewCommand {
    /// Show the latest authoritative settlement receipt.
    Show {
        session: String,
        transaction: String,
    },
    /// Accept a fully validated change transaction.
    Accept {
        session: String,
        transaction: String,
    },
    /// Return a change transaction to repair.
    Repair {
        session: String,
        transaction: String,
    },
    /// Restore the transaction baseline when its coverage permits rollback.
    Rollback {
        session: String,
        transaction: String,
        /// Acknowledge residual side-effect risk for best-effort rollback.
        #[arg(long)]
        acknowledge_risk: bool,
    },
}

pub(crate) async fn run(
    command: &ReviewCommand,
    explicit_socket: Option<PathBuf>,
) -> anyhow::Result<()> {
    let mut client = ReviewRpcClient::connect(explicit_socket).await?;
    let result = client.request(request(command)).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn request(command: &ReviewCommand) -> ClientRpcRequest {
    match command {
        ReviewCommand::Show {
            session,
            transaction,
        } => ClientRpcRequest::TransactionSettlementGet(fabric::TransactionSettlementGetParams {
            session_id: session.clone(),
            transaction_id: transaction.clone(),
        }),
        ReviewCommand::Accept {
            session,
            transaction,
        } => review_request(
            session,
            transaction,
            fabric::TransactionReviewAction::Accept,
            false,
        ),
        ReviewCommand::Repair {
            session,
            transaction,
        } => review_request(
            session,
            transaction,
            fabric::TransactionReviewAction::Repair,
            false,
        ),
        ReviewCommand::Rollback {
            session,
            transaction,
            acknowledge_risk,
        } => review_request(
            session,
            transaction,
            fabric::TransactionReviewAction::Rollback,
            *acknowledge_risk,
        ),
    }
}

fn review_request(
    session: &str,
    transaction: &str,
    action: fabric::TransactionReviewAction,
    risk_acknowledged: bool,
) -> ClientRpcRequest {
    ClientRpcRequest::TransactionReview(fabric::TransactionReviewParams {
        session_id: session.to_owned(),
        transaction_id: transaction.to_owned(),
        action,
        risk_acknowledged,
    })
}

struct ReviewRpcClient {
    stream: BufStream<UnixStream>,
    next_id: u64,
}

impl ReviewRpcClient {
    async fn connect(explicit_socket: Option<PathBuf>) -> anyhow::Result<Self> {
        let socket = interact::host::ensure_user_socket(explicit_socket).await?;
        let stream = UnixStream::connect(&socket)
            .await
            .map_err(|error| anyhow::anyhow!("connecting {}: {error}", socket.display()))?;
        Ok(Self {
            stream: BufStream::new(stream),
            next_id: 1,
        })
    }

    async fn request(&mut self, request: ClientRpcRequest) -> anyhow::Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.stream
            .write_all(request.to_json_rpc(Some(id))?.to_string().as_bytes())
            .await?;
        self.stream.write_all(b"\n").await?;
        self.stream.flush().await?;
        for _ in 0..32 {
            let mut line = String::new();
            anyhow::ensure!(
                self.stream.read_line(&mut line).await? > 0,
                "daemon closed the review control connection"
            );
            let response: serde_json::Value = serde_json::from_str(line.trim())?;
            if response.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
                if let Some(error) = response.get("error") {
                    anyhow::bail!(
                        "daemon review request failed: {}",
                        error["message"].as_str().unwrap_or("missing result")
                    );
                }
                return response
                    .get("result")
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("daemon review response omitted result"));
            }
        }
        anyhow::bail!("too many unrelated daemon messages on review control connection")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_requires_an_explicit_typed_risk_flag() {
        let request = request(&ReviewCommand::Rollback {
            session: "session-a".into(),
            transaction: uuid::Uuid::nil().to_string(),
            acknowledge_risk: true,
        });
        let ClientRpcRequest::TransactionReview(params) = request else {
            panic!("rollback must use the Host transaction review RPC");
        };
        assert_eq!(params.action, fabric::TransactionReviewAction::Rollback);
        assert!(params.risk_acknowledged);
    }
}
