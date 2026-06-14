//! Evaluator — scores candidate runtimes after sandbox testing.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use aletheon_abi::{RuntimeCandidate, TestResult, Evaluation};

pub struct Evaluator;

impl Evaluator {
    pub fn new() -> Self { Self }

    /// Evaluate a candidate based on test results.
    pub async fn evaluate(&self, _candidate: &RuntimeCandidate, _test: &TestResult) -> Result<Evaluation> {
        todo!("Evaluator: evaluate not yet implemented")
    }
}
