//! Executive-side capability implementations that still depend on
//! executive internals (Gmail ingestion depends on the artifact store and
//! goal migrations). Neutral capability handlers now live in `gateway::handlers`.

pub mod gmail_ingest;
