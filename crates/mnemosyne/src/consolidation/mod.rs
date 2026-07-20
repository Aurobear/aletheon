mod consolidator;
mod extractor;
mod migrations;
mod repository;

pub use consolidator::{ConsolidationDecision, ConsolidationOutcome, ScopedConsolidator};
pub use extractor::{CandidateExtractor, CanonicalMemoryEvent, ExtractionBatch};
pub use repository::{
    ConsolidationRepository, ExtractionCompletion, ExtractionJob, ExtractionStatus,
    LeasedExtraction, MemoryCandidate, ScopeLease,
};
