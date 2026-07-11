use anyhow::Result;
use fabric::{TurnResult, TurnStop};

#[derive(Debug, Clone, Default)]
pub struct PostTurnPipeline;

impl PostTurnPipeline {
    pub async fn run(&self, result: TurnResult) -> Result<TurnResult> {
        match result.stop {
            TurnStop::Completed | TurnStop::Blocked | TurnStop::Cancelled | TurnStop::Failed => {
                Ok(result)
            }
        }
    }
}
