//! Spec editor — modifies genome specifications.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use crate::core::types::{Genome, GenomePatch};

pub struct SpecEditor;

impl SpecEditor {
    pub fn new() -> Self { Self }

    /// Apply a patch to the genome.
    pub async fn apply_patch(&self, _genome: &mut Genome, _patch: &GenomePatch) -> Result<()> {
        todo!("SpecEditor: apply_patch not yet implemented")
    }
}
