use anyhow::Result;
use fabric::{RecallRequest, TurnRequest, TurnServices};

#[derive(Debug, Clone, Default)]
pub struct PreTurnPipeline;

impl PreTurnPipeline {
    pub async fn run(
        &self,
        mut request: TurnRequest,
        services: &dyn TurnServices,
    ) -> Result<TurnRequest> {
        let recall = services
            .recall(RecallRequest {
                session_id: request.session_id.clone(),
                input: request.input.clone(),
            })
            .await?;
        if !recall.snippets.is_empty() {
            request.input = format!(
                "<memory>\n{}\n</memory>\n\n{}",
                recall.snippets.join("\n"),
                request.input
            );
        }
        Ok(request)
    }
}
