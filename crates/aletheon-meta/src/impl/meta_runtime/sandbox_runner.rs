//! Sandbox runner — tests candidate runtimes in isolation.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use aletheon_abi::{RuntimeCandidate, TestResult};

pub struct SandboxRunner;

impl SandboxRunner {
    pub fn new() -> Self { Self }

    /// Run sandbox tests on a candidate runtime.
    pub async fn run_tests(&self, _candidate: &RuntimeCandidate) -> Result<TestResult> {
        todo!("SandboxRunner: run_tests not yet implemented")
    }
}
