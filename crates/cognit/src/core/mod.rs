//! Focused cognitive algorithms used by production ReAct/Linear harnesses.
//!
//! Composition belongs to the harness/session boundary. The former `CognitCore`
//! aggregate had no production constructor and duplicated that composition path.

pub mod awareness;
pub mod awareness_signal;
pub mod claim_auditor;
pub mod cognitive_task;
pub mod critic;
pub mod evidence;
pub mod evolution_trigger;
pub mod experience_summarizer;
pub mod learner;
pub mod planner;
pub mod progress_auditor;
pub mod reasoner;
pub mod reflector;
pub mod skill_extractor;
pub mod task_decomposition;
pub mod world_model;

pub use self::experience_summarizer::ExperienceSummarizer;
pub use claim_auditor::*;
pub use cognitive_task::*;
pub use evidence::*;
pub use progress_auditor::*;
pub use task_decomposition::*;
