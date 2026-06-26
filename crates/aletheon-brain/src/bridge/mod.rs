//! Bridge layer — adapts core infrastructure into BrainCore.

pub mod llm;
pub mod dual_model;
pub mod inference;
pub mod learning;

pub use llm::LlmBridge;
pub use dual_model::{DualModelBridge, DualModelConfig, TaskComplexity};
pub use inference::InferenceBridge;
pub use learning::LearningBridge;
