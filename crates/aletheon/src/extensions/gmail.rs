//! Bounded classification contracts for the Gmail extension ingress.
//!
//! The classification logic moved to the `adapters-google` crate alongside the
//! Gmail channel; this host extension surface re-exports it so the extension
//! package path stays stable.

pub use adapters_google::gmail_classification::{classify_verified_subject, GmailClassification};
