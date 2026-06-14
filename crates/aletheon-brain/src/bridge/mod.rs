//! Bridge layer — adapts core infrastructure into BrainCore.

pub mod llm;
pub mod inference;
pub mod learning;

pub use llm::LlmBridge;
pub use inference::InferenceBridge;
pub use learning::LearningBridge;
