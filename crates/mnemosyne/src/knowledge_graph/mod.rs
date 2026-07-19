mod entity;
mod extract;
mod graph;
mod relation;

pub use entity::{Entity, EntityId, EntityType};
pub use extract::{extract_entities_from_content, infer_relations};
pub use graph::KnowledgeGraph;
pub use relation::{Relation, RelationType};
