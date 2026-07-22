//! Provider-neutral coding attempt contracts consumed by Goal orchestration.

use fabric::CodingJobSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingAttemptRequest {
    pub job: CodingJobSpec,
    pub task_input: String,
}
