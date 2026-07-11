//! Bridge layer — adapts core infrastructure into CognitCore.

pub mod dual_model;
pub mod inference;
pub mod learning;
pub mod llm;

pub use dual_model::{DualModelBridge, DualModelConfig, TaskComplexity};
pub use inference::InferenceBridge;
pub use learning::LearningBridge;
pub use llm::LlmBridge;
