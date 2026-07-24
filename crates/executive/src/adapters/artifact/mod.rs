//! Content-addressed external artifact persistence.

pub mod store;

pub use store::{
    ArtifactMetadata, ArtifactRecord, ArtifactScanStatus, ArtifactStore, ArtifactWriter,
};
