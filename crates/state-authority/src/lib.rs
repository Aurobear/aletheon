//! State authority — single source of truth for every durable fact type (Wave 4).
//! StorageManifest, schema migrations, reconciliation, and kill-9 recovery.

pub mod manifest;
pub mod authority;
pub mod migration;
pub mod trajectory;

pub use manifest::{StorageManifest, StoreRole, AuthorityId, StoreKind};
pub use authority::{Authority, FactKind, DurableFact};
pub use migration::{MigrationCoordinator, SchemaVersion};
pub use trajectory::{TrajectoryReader, TurnRecord, ToolCallRecord};
