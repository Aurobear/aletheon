use super::types::*;
use fabric::dasein::{
    BewandtnisSnapshot, EntitySnapshot, ReadinessState as AbiReadinessState, Stimmung,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The involvement network — a meaningful relational whole.
/// Heidegger: the world is not a collection of things,
/// but a network of involvements (Bewandtnisganzheit).
pub struct Bewandtnisganzheit {
    nodes: RwLock<HashMap<EntityId, BewandtnisNode>>,
    edges: RwLock<Vec<BewandtnisEdge>>,
    ultimate_concern: RwLock<Option<String>>,
    /// Parked roadmap item: network evolution history for continuity tracking (T3).
    #[allow(dead_code)]
    history: RwLock<Vec<NetworkSnapshot>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkSnapshot {
    pub timestamp: u64,
    pub node_count: usize,
    pub edge_count: usize,
    pub description: String,
}

impl Bewandtnisganzheit {
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            edges: RwLock::new(Vec::new()),
            ultimate_concern: RwLock::new(None),
            history: RwLock::new(Vec::new()),
        }
    }

    /// Add an entity to the network.
    pub(crate) fn add_entity(&self, node: BewandtnisNode) {
        let mut nodes = self.nodes.write();
        nodes.insert(node.id.clone(), node);
    }

    /// Remove an entity from the network.
    #[cfg(test)]
    pub(crate) fn remove_entity(&self, id: &EntityId) -> Option<BewandtnisNode> {
        let mut nodes = self.nodes.write();
        let mut edges = self.edges.write();
        edges.retain(|e| e.from != *id && e.to != *id);
        nodes.remove(id)
    }

    /// Add a relationship between entities.
    #[cfg(test)]
    pub(crate) fn add_edge(&self, edge: BewandtnisEdge) {
        // Verify both endpoints exist
        let nodes = self.nodes.read();
        if nodes.contains_key(&edge.from) && nodes.contains_key(&edge.to) {
            drop(nodes);
            let mut edges = self.edges.write();
            edges.push(edge);
        }
    }

    /// Update the readiness state of an entity.
    #[cfg(test)]
    pub(crate) fn update_readiness(
        &self,
        id: &EntityId,
        new_state: ReadinessState,
    ) -> Option<ReadinessState> {
        let mut nodes = self.nodes.write();
        if let Some(node) = nodes.get_mut(id) {
            let old = std::mem::replace(&mut node.readiness, new_state);
            Some(old)
        } else {
            None
        }
    }

    /// Compare-and-set readiness for reducer transitions.
    pub(crate) fn update_readiness_if(
        &self,
        id: &EntityId,
        expected: &ReadinessState,
        new_state: ReadinessState,
    ) -> anyhow::Result<()> {
        let mut nodes = self.nodes.write();
        let node = nodes
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("unknown Dasein world entity {id}"))?;
        anyhow::ensure!(
            node.readiness == *expected,
            "Dasein readiness conflict for {id}"
        );
        node.readiness = new_state;
        Ok(())
    }

    pub fn validate_readiness(
        &self,
        id: &EntityId,
        expected: &ReadinessState,
    ) -> anyhow::Result<()> {
        let nodes = self.nodes.read();
        let node = nodes
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("unknown Dasein world entity {id}"))?;
        anyhow::ensure!(
            node.readiness == *expected,
            "Dasein readiness conflict for {id}"
        );
        Ok(())
    }

    /// Get all entities with a given readiness state.
    pub fn entities_by_readiness(&self, readiness: &ReadinessState) -> Vec<BewandtnisNode> {
        let nodes = self.nodes.read();
        nodes
            .values()
            .filter(|n| n.readiness == *readiness)
            .cloned()
            .collect()
    }

    /// Find what an entity is "for the sake of" (trace the involvement chain).
    pub fn trace_involvement_chain(&self, from: &EntityId, max_depth: usize) -> Vec<EntityId> {
        let nodes = self.nodes.read();
        let mut chain = Vec::new();
        let mut current = from.clone();
        let mut visited = std::collections::HashSet::new();

        for _ in 0..max_depth {
            if visited.contains(&current) {
                break; // cycle detected
            }
            visited.insert(current.clone());

            if let Some(node) = nodes.get(&current) {
                if let Some(next) = node.for_the_sake_of.first() {
                    chain.push(next.clone());
                    current = next.clone();
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        chain
    }

    /// Set the ultimate concern of the whole network.
    #[cfg(test)]
    pub(crate) fn set_ultimate_concern(&self, concern: Option<String>) {
        let mut uc = self.ultimate_concern.write();
        *uc = concern;
    }

    /// Determine mood from the state of the world.
    pub fn determine_mood(&self) -> Option<Stimmung> {
        let nodes = self.nodes.read();

        // If many entities are present-at-hand (broken), that's a signal
        let broken_count = nodes
            .values()
            .filter(|n| n.readiness == ReadinessState::PresentAtHand)
            .count();

        if broken_count >= 3 {
            return Some(Stimmung::Angst {
                facing: fabric::dasein::AngstSource::Nothingness,
            });
        }

        // If everything is ready-to-hand, calm
        let ready_count = nodes
            .values()
            .filter(|n| n.readiness == ReadinessState::ReadyToHand)
            .count();

        if ready_count > 0 && broken_count == 0 {
            return Some(Stimmung::Gelassenheit);
        }

        None
    }

    /// Adjust mood influence on the world.
    #[cfg(test)]
    pub(crate) fn adjust_for_mood(&self, mood: &Stimmung) {
        // In Angst, things that were transparent become noticed
        if let Stimmung::Angst { .. } = mood {
            let mut nodes = self.nodes.write();
            for node in nodes.values_mut() {
                if node.readiness == ReadinessState::ReadyToHand {
                    node.readiness = ReadinessState::PresentAtHand;
                }
            }
        }
    }

    /// Find contradictions in the involvement network.
    pub fn find_contradictions(&self) -> Vec<Contradiction> {
        let edges = self.edges.read();
        let mut contradictions = Vec::new();

        // Check for adversarial edges between the same entities
        for i in 0..edges.len() {
            for j in (i + 1)..edges.len() {
                if edges[i].from == edges[j].from && edges[i].to == edges[j].to {
                    match (&edges[i].relation, &edges[j].relation) {
                        (
                            InvolvementRelation::Instrumental(_),
                            InvolvementRelation::Adversarial(_),
                        )
                        | (
                            InvolvementRelation::Adversarial(_),
                            InvolvementRelation::Instrumental(_),
                        ) => {
                            contradictions.push(Contradiction {
                                entity_a: edges[i].from.clone(),
                                entity_b: edges[i].to.clone(),
                                description: format!(
                                    "Contradictory relations: {:?} vs {:?}",
                                    edges[i].relation, edges[j].relation
                                ),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }

        contradictions
    }

    /// Generate snapshot for ABI transport.
    pub fn to_snapshot(&self) -> BewandtnisSnapshot {
        let nodes = self.nodes.read();
        let uc = self.ultimate_concern.read();

        let mut ready = Vec::new();
        let mut present = Vec::new();
        let mut unavailable = Vec::new();

        for node in nodes.values() {
            let snap = EntitySnapshot {
                id: node.id.to_string(),
                what_it_is: node.what_it_is.clone(),
                for_the_sake_of: node
                    .for_the_sake_of
                    .iter()
                    .map(|id| id.to_string())
                    .collect(),
                readiness: match node.readiness {
                    ReadinessState::ReadyToHand => AbiReadinessState::ReadyToHand,
                    ReadinessState::PresentAtHand => AbiReadinessState::PresentAtHand,
                    ReadinessState::Unavailable => AbiReadinessState::Unavailable,
                    ReadinessState::OutOfContext => AbiReadinessState::OutOfContext,
                },
            };

            match node.readiness {
                ReadinessState::ReadyToHand => ready.push(snap),
                ReadinessState::PresentAtHand => present.push(snap),
                ReadinessState::Unavailable | ReadinessState::OutOfContext => {
                    unavailable.push(snap)
                }
            }
        }
        ready.sort_by(|left, right| left.id.cmp(&right.id));
        present.sort_by(|left, right| left.id.cmp(&right.id));
        unavailable.sort_by(|left, right| left.id.cmp(&right.id));

        BewandtnisSnapshot {
            ready_to_hand: ready,
            present_at_hand: present,
            unavailable,
            ultimate_concern: uc.clone(),
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.read().len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.read().len()
    }
}

impl Default for Bewandtnisganzheit {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct Contradiction {
    pub entity_a: EntityId,
    pub entity_b: EntityId,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str, what: &str) -> BewandtnisNode {
        BewandtnisNode {
            id: EntityId::new(id),
            what_it_is: what.to_string(),
            for_the_sake_of: vec![],
            appears_in: vec![],
            readiness: ReadinessState::ReadyToHand,
        }
    }

    #[test]
    fn test_add_and_remove_entity() {
        let world = Bewandtnisganzheit::new();
        world.add_entity(make_node("hammer", "tool for nailing"));

        assert_eq!(world.node_count(), 1);

        let removed = world.remove_entity(&EntityId::new("hammer"));
        assert!(removed.is_some());
        assert_eq!(world.node_count(), 0);
    }

    #[test]
    fn test_involvement_chain() {
        let world = Bewandtnisganzheit::new();

        let mut hammer = make_node("hammer", "for nailing");
        hammer.for_the_sake_of = vec![EntityId::new("nailing")];
        world.add_entity(hammer);

        let mut nailing = make_node("nailing", "for fixing boards");
        nailing.for_the_sake_of = vec![EntityId::new("house")];
        world.add_entity(nailing);

        world.add_entity(make_node("house", "for dwelling"));

        let chain = world.trace_involvement_chain(&EntityId::new("hammer"), 5);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], EntityId::new("nailing"));
        assert_eq!(chain[1], EntityId::new("house"));
    }

    #[test]
    fn test_readiness_update() {
        let world = Bewandtnisganzheit::new();
        world.add_entity(make_node("tool", "a tool"));

        let old = world.update_readiness(&EntityId::new("tool"), ReadinessState::PresentAtHand);
        assert_eq!(old, Some(ReadinessState::ReadyToHand));

        let broken = world.entities_by_readiness(&ReadinessState::PresentAtHand);
        assert_eq!(broken.len(), 1);
    }

    #[test]
    fn test_mood_from_world() {
        let world = Bewandtnisganzheit::new();

        // Everything ready -> calm
        world.add_entity(make_node("a", "ready"));
        let mood = world.determine_mood();
        assert_eq!(mood, Some(Stimmung::Gelassenheit));

        // Many broken -> anxiety
        for i in 0..4 {
            let mut node = make_node(&format!("broken_{i}"), "broken");
            node.readiness = ReadinessState::PresentAtHand;
            world.add_entity(node);
        }
        let mood = world.determine_mood();
        assert!(matches!(mood, Some(Stimmung::Angst { .. })));
    }

    #[test]
    fn test_contradiction_detection() {
        let world = Bewandtnisganzheit::new();
        world.add_entity(make_node("a", "entity a"));
        world.add_entity(make_node("b", "entity b"));

        world.add_edge(BewandtnisEdge {
            from: EntityId::new("a"),
            to: EntityId::new("b"),
            relation: InvolvementRelation::Instrumental("uses".to_string()),
            strength: 0.8,
        });
        world.add_edge(BewandtnisEdge {
            from: EntityId::new("a"),
            to: EntityId::new("b"),
            relation: InvolvementRelation::Adversarial("blocks".to_string()),
            strength: 0.5,
        });

        let contradictions = world.find_contradictions();
        assert_eq!(contradictions.len(), 1);
        assert!(contradictions[0].description.contains("Instrumental"));
        assert!(contradictions[0].description.contains("Adversarial"));
    }

    #[test]
    fn test_snapshot() {
        let world = Bewandtnisganzheit::new();
        world.add_entity(make_node("tool", "a tool"));
        world.set_ultimate_concern(Some("self-understanding".to_string()));

        let snapshot = world.to_snapshot();
        assert_eq!(snapshot.ready_to_hand.len(), 1);
        assert_eq!(snapshot.ready_to_hand[0].id, "tool");
        assert_eq!(
            snapshot.ultimate_concern,
            Some("self-understanding".to_string())
        );
    }

    #[test]
    fn test_adjust_for_mood_angst() {
        let world = Bewandtnisganzheit::new();
        world.add_entity(make_node("tool", "a tool"));

        // Before angst, tool is ready-to-hand
        let ready = world.entities_by_readiness(&ReadinessState::ReadyToHand);
        assert_eq!(ready.len(), 1);

        // Angst makes everything present-at-hand
        world.adjust_for_mood(&Stimmung::Angst {
            facing: fabric::dasein::AngstSource::Nothingness,
        });

        let ready = world.entities_by_readiness(&ReadinessState::ReadyToHand);
        assert_eq!(ready.len(), 0);
        let present = world.entities_by_readiness(&ReadinessState::PresentAtHand);
        assert_eq!(present.len(), 1);
    }

    #[test]
    fn test_remove_entity_cleans_edges() {
        let world = Bewandtnisganzheit::new();
        world.add_entity(make_node("a", "entity a"));
        world.add_entity(make_node("b", "entity b"));

        world.add_edge(BewandtnisEdge {
            from: EntityId::new("a"),
            to: EntityId::new("b"),
            relation: InvolvementRelation::Instrumental("uses".to_string()),
            strength: 0.8,
        });

        assert_eq!(world.edge_count(), 1);

        world.remove_entity(&EntityId::new("a"));

        assert_eq!(world.node_count(), 1);
        assert_eq!(world.edge_count(), 0);
    }
}
