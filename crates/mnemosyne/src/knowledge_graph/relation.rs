use serde::{Deserialize, Serialize};

use super::entity::EntityId;

/// Typed edge between two entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub from_entity: EntityId,
    pub to_entity: EntityId,
    pub relation_type: RelationType,
    /// 0.0-1.0 confidence from the extraction heuristic.
    pub confidence: f64,
    /// Which MemoryRecord produced this edge.
    pub provenance: String,
}

impl Relation {
    pub fn new(
        from: EntityId,
        to: EntityId,
        relation_type: RelationType,
        confidence: f64,
        provenance: String,
    ) -> Self {
        Self {
            from_entity: from,
            to_entity: to,
            relation_type,
            confidence,
            provenance,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    Founded,
    InvestedIn,
    Advises,
    WorksAt,
    Attended,
    Mentions,
    RelatedTo,
    /// Catch-all for edges from regex-link extraction where no verb heuristic fired.
    Linked,
}
