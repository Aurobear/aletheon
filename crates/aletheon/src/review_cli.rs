//! Host transaction review CLI projection.
//!
//! The command only submits typed actions and renders authoritative daemon
//! receipts. It does not inspect the workspace or derive settlement locally.

use std::path::PathBuf;

use clap::Subcommand;
use gateway::client::{CommandOutcome, GatewayClient, UnixSocketTransport};
use gateway::protocol::{
    Command, Query, ReviewTransactionRequest, SessionRef, TransactionSettlementQuery,
};

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
    let result = client.request(command).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

struct ReviewRpcClient {
    client: GatewayClient<UnixSocketTransport>,
}

impl ReviewRpcClient {
    async fn connect(explicit_socket: Option<PathBuf>) -> anyhow::Result<Self> {
        let socket = crate::ensure_user_socket(explicit_socket).await?;
        let transport = UnixSocketTransport::connect(&socket)
            .await
            .map_err(|error| anyhow::anyhow!("connecting {}: {error}", socket.display()))?;
        Ok(Self {
            client: GatewayClient::new(transport),
        })
    }

    async fn request(&mut self, command: &ReviewCommand) -> anyhow::Result<serde_json::Value> {
        match command {
            ReviewCommand::Show {
                session,
                transaction,
            } => self
                .client
                .query(Query::TransactionSettlement(TransactionSettlementQuery {
                    session: SessionRef(session.clone()),
                    transaction: transaction.clone(),
                }))
                .await
                .map_err(|error| anyhow::anyhow!("daemon review query failed: {error}")),
            ReviewCommand::Accept {
                session,
                transaction,
            }
            | ReviewCommand::Repair {
                session,
                transaction,
            }
            | ReviewCommand::Rollback {
                session,
                transaction,
                ..
            } => {
                let (action, acknowledge_risk) = match command {
                    ReviewCommand::Accept { .. } => {
                        (::contracts::TransactionReviewAction::Accept, false)
                    }
                    ReviewCommand::Repair { .. } => {
                        (::contracts::TransactionReviewAction::Repair, false)
                    }
                    ReviewCommand::Rollback {
                        acknowledge_risk, ..
                    } => (
                        ::contracts::TransactionReviewAction::Rollback,
                        *acknowledge_risk,
                    ),
                    ReviewCommand::Show { .. } => unreachable!(),
                };
                match self
                    .client
                    .send(Command::ReviewTransaction(ReviewTransactionRequest {
                        session: SessionRef(session.clone()),
                        transaction: transaction.clone(),
                        action,
                        acknowledge_risk,
                    }))
                    .await
                    .map_err(|error| anyhow::anyhow!("daemon review command failed: {error}"))?
                {
                    CommandOutcome::TransactionReviewed { outcome } => Ok(outcome),
                    other => anyhow::bail!("daemon review returned unexpected receipt: {other:?}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_requires_an_explicit_typed_risk_flag() {
        let command = ReviewCommand::Rollback {
            session: "session-a".into(),
            transaction: uuid::Uuid::nil().to_string(),
            acknowledge_risk: true,
        };
        assert!(matches!(
            command,
            ReviewCommand::Rollback {
                acknowledge_risk: true,
                ..
            }
        ));
    }
}
