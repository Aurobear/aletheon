//! Deprecated compatibility facade for stable inference contracts.

pub mod provider {
    pub use crate::adapters::inference::provider::*;
}
pub use provider::*;

pub mod pulse {
    pub use crate::adapters::inference::pulse::*;
}
pub use pulse::*;

pub mod scheduler {
    pub use crate::adapters::inference::scheduler::*;
}
pub use scheduler::*;

#[deprecated(note = "use cognit::composition::inference_factory")]
pub mod provider_factory {
    pub use crate::composition::inference_factory::*;
}
