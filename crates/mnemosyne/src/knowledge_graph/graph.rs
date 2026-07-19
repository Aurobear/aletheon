use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::entity::{Entity, EntityId};
use super::relation::{Relation, RelationType};

/// In-memory knowledge graph. Stores entities and typed edges.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    entities: HashMap<EntityId, Entity>,
    relations: Vec<Relation>,
    /// Index: entity -> indices into relations (outgoing)
    outgoing: HashMap<EntityId, Vec<usize>>,
    /// Index: entity -> indices into relations (incoming)
    incoming: HashMap<EntityId, Vec<usize>>,
}

impl KnowledgeGraph {
    /// Insert or update an entity.
    pub fn upsert_entity(&mut self, entity: Entity) -> EntityId {
        let id = entity.id.clone();
        self.entities.insert(id.clone(), entity);
        id
    }

    /// Get an entity by ID.
    pub fn get_entity(&self, id: &EntityId) -> Option<&Entity> {
        self.entities.get(id)
    }

    /// Find an entity by name (case-insensitive).
    pub fn find_entity_by_name(&self, name: &str) -> Option<&Entity> {
        self.entities.values().find(|e| e.matches_name(name))
    }

    /// Add a typed relation between two entities. Idempotent (won't duplicate).
    pub fn add_relation(&mut self, relation: Relation) {
        let from = relation.from_entity.clone();
        let to = relation.to_entity.clone();

        // Skip if duplicate
        let is_duplicate = self.relations.iter().any(|r| {
            r.from_entity == from && r.to_entity == to && r.relation_type == relation.relation_type
        });

        if !is_duplicate {
            let idx = self.relations.len();
            self.relations.push(relation);
            self.outgoing.entry(from).or_default().push(idx);
            self.incoming.entry(to).or_default().push(idx);
        }
    }

    /// Get all relations originating from an entity.
    pub fn outgoing_relations(&self, entity: &EntityId) -> Vec<&Relation> {
        self.outgoing
            .get(entity)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&i| self.relations.get(i))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all relations incoming to an entity.
    pub fn incoming_relations(&self, entity: &EntityId) -> Vec<&Relation> {
        self.incoming
            .get(entity)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&i| self.relations.get(i))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// All neighbors (outgoing relations + the target entities).
    pub fn neighbors(&self, entity: &EntityId) -> Vec<(&Entity, &Relation)> {
        self.outgoing_relations(entity)
            .into_iter()
            .filter_map(|rel| {
                self.entities
                    .get(&rel.to_entity)
                    .map(|target| (target, rel))
            })
            .collect()
    }

    /// BFS path finding between two entities.
    pub fn find_path(
        &self,
        from: &EntityId,
        to: &EntityId,
        max_depth: usize,
    ) -> Option<Vec<(EntityId, RelationType)>> {
        if from == to {
            return Some(Vec::new());
        }

        let mut visited: HashSet<EntityId> = HashSet::new();
        let mut queue: VecDeque<(EntityId, Vec<(EntityId, RelationType)>)> = VecDeque::new();

        visited.insert(from.clone());
        queue.push_back((from.clone(), Vec::new()));

        while let Some((current, path)) = queue.pop_front() {
            if path.len() >= max_depth {
                continue;
            }

            for rel in self.outgoing_relations(&current) {
                if !visited.contains(&rel.to_entity) {
                    let mut new_path = path.clone();
                    new_path.push((rel.to_entity.clone(), rel.relation_type));
                    if &rel.to_entity == to {
                        return Some(new_path);
                    }
                    visited.insert(rel.to_entity.clone());
                    queue.push_back((rel.to_entity.clone(), new_path));
                }
            }
        }

        None
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn relation_count(&self) -> usize {
        self.relations.len()
    }

    /// Entities — relations they have to other entities.
    pub fn related_entities(&self, entity: &EntityId) -> Vec<(&Entity, &Relation)> {
        self.neighbors(entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntityType;

    fn make_entity(name: &str, etype: EntityType) -> Entity {
        Entity::new(name.into(), etype, "test".into())
    }

    #[test]
    fn upsert_idempotent() {
        let mut kg = KnowledgeGraph::default();
        let alice = make_entity("Alice Chen", EntityType::Person);
        let id = kg.upsert_entity(alice.clone());
        kg.upsert_entity(alice);
        assert_eq!(kg.entity_count(), 1);
        assert_eq!(id, EntityId::derive("Alice Chen"));
    }

    #[test]
    fn neighbors_and_path() {
        let mut kg = KnowledgeGraph::default();
        let alice_id = kg.upsert_entity(make_entity("Alice Chen", EntityType::Person));
        let acme_id = kg.upsert_entity(make_entity("Acme Corp", EntityType::Company));

        kg.add_relation(Relation::new(
            alice_id.clone(),
            acme_id.clone(),
            RelationType::WorksAt,
            0.8,
            "test".into(),
        ));

        let neighbors = kg.neighbors(&alice_id);
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].0.name, "Acme Corp");
        assert_eq!(neighbors[0].1.relation_type, RelationType::WorksAt);

        let path = kg.find_path(&alice_id, &acme_id, 3);
        assert!(path.is_some());
        assert_eq!(path.unwrap().len(), 1);
    }

    #[test]
    fn relation_dedup() {
        let mut kg = KnowledgeGraph::default();
        let a = kg.upsert_entity(make_entity("Alice", EntityType::Person));
        let b = kg.upsert_entity(make_entity("Bob", EntityType::Person));
        kg.add_relation(Relation::new(
            a.clone(),
            b.clone(),
            RelationType::Mentions,
            0.5,
            "test".into(),
        ));
        kg.add_relation(Relation::new(
            a.clone(),
            b.clone(),
            RelationType::Mentions,
            0.5,
            "test".into(),
        ));
        assert_eq!(kg.relation_count(), 1);
    }

    #[test]
    fn entity_find_by_name() {
        let mut kg = KnowledgeGraph::default();
        kg.upsert_entity(make_entity("Acme Corp", EntityType::Company));
        let found = kg.find_entity_by_name("acme corp");
        assert!(found.is_some());
        assert!(kg.find_entity_by_name("nonexistent").is_none());
    }
}
