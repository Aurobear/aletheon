use anyhow::Result;
use fabric::{TurnRequest, TurnServices};

#[derive(Debug, Clone, Default)]
pub struct PreTurnPipeline;

impl PreTurnPipeline {
    pub async fn run(
        &self,
        request: TurnRequest,
        _services: &dyn TurnServices,
    ) -> Result<TurnRequest> {
        // Memory is projected only through Mnemosyne -> Agora candidates ->
        // conscious context. The legacy TurnServices recall hook remains a
        // compatibility contract but is intentionally not prompt-injected.
        Ok(request)
    }
}
