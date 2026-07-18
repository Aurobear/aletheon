use regex::Regex;

use super::entity::{Entity, EntityType};
use super::relation::{Relation, RelationType};

fn relation_type_key(rt: &RelationType) -> u8 {
    match rt {
        RelationType::Founded => 0,
        RelationType::InvestedIn => 1,
        RelationType::Advises => 2,
        RelationType::WorksAt => 3,
        RelationType::Attended => 4,
        RelationType::Mentions => 5,
        RelationType::RelatedTo => 6,
        RelationType::Linked => 7,
    }
}

/// Extract entities from content using regex patterns.
/// Reuses patterns from FactStore::extract_entities where applicable.
pub fn extract_entities_from_content(content: &str, provenance: &str) -> Vec<Entity> {
    let mut entities = Vec::new();

    // Capitalized multi-word: two or more consecutive capitalized words
    let cap_re = Regex::new(r"\b([A-Z][a-z]+(?:\s+[A-Z][a-z]+)+)\b").unwrap();
    for cap in cap_re.captures_iter(content) {
        let name = cap[1].to_string();
        entities.push(Entity::new(name, EntityType::Other, provenance.to_string()));
    }

    // Markdown links: [Name](path/...)
    let md_link_re = Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap();
    for cap in md_link_re.captures_iter(content) {
        let name = cap[1].to_string();
        if name.len() >= 2 && name.chars().any(|c| c.is_alphabetic()) {
            entities.push(Entity::new(name, EntityType::Other, provenance.to_string()));
        }
    }

    // Wikilinks: [[Name]]
    let wiki_re = Regex::new(r"\[\[([^\]]+)\]\]").unwrap();
    for cap in wiki_re.captures_iter(content) {
        let name = cap[1].to_string();
        if let Some(pipe_pos) = name.find('|') {
            let display = name[pipe_pos + 1..].to_string();
            entities.push(Entity::new(
                display,
                EntityType::Other,
                provenance.to_string(),
            ));
        } else if name.len() >= 2 {
            entities.push(Entity::new(name, EntityType::Other, provenance.to_string()));
        }
    }

    // "X aka Y" pattern
    let aka_re = Regex::new(r"\b(\w[\w\s]+?)\s+aka\s+(\w[\w\s]+?)(?:\s|,|\.|$)").unwrap();
    for cap in aka_re.captures_iter(content) {
        let a = cap[1].trim().to_string();
        let b = cap[2].trim().to_string();
        if !a.is_empty() {
            entities.push(Entity::new(a, EntityType::Other, provenance.to_string()));
        }
        if !b.is_empty() {
            entities.push(Entity::new(b, EntityType::Other, provenance.to_string()));
        }
    }

    // Basic entity type classification
    for entity in &mut entities {
        let name_lower = entity.name.to_lowercase();
        if name_lower.contains(" inc")
            || name_lower.contains(" corp")
            || name_lower.contains(" llc")
            || name_lower.contains(" ltd")
        {
            entity.entity_type = EntityType::Company;
        } else if name_lower.contains(" university")
            || name_lower.contains(" college")
            || name_lower.contains(" school")
        {
            entity.entity_type = EntityType::Organization;
        }
    }

    // Deduplicate by name
    entities.sort_by(|a, b| a.name.cmp(&b.name));
    entities.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    entities
}

/// Heuristic verb patterns for relation type inference.
const FOUNDED_VERBS: &[&str] = &["founded", "co-founded", "started"];
const INVESTED_VERBS: &[&str] = &["invested in", "backed", "funded"];
const ADVISES_VERBS: &[&str] = &[
    "advises",
    "advisor to",
    "board member at",
    "board member of",
];
const WORKS_AT_VERBS: &[&str] = &["works at", "engineer at", "joined", "employed by"];
const ATTENDED_VERBS: &[&str] = &["attended", "graduated from", "studied at", "alumnus of"];
const MENTIONS_VERBS: &[&str] = &["met", "talked to", "emailed", "spoke with", "met with"];

/// Infer typed relations from sentence context surrounding entity mentions.
pub fn infer_relations(content: &str, entities: &[Entity], provenance: &str) -> Vec<Relation> {
    let mut relations = Vec::new();

    // For each entity pair, check if they co-occur in the same sentence
    let sentences: Vec<&str> = content
        .split_inclusive(&['.', '!', '?', '\n'][..])
        .filter(|s| !s.trim().is_empty())
        .collect();

    for sentence in &sentences {
        let sentence_lower = sentence.to_lowercase();

        // Find entities mentioned in this sentence
        let mentioned: Vec<&Entity> = entities
            .iter()
            .filter(|e| sentence_lower.contains(&e.name.to_lowercase()))
            .collect();

        if mentioned.len() < 2 {
            continue;
        }

        // Try to infer relation type from verbs
        let relation_type = if FOUNDED_VERBS.iter().any(|v| sentence_lower.contains(v)) {
            Some(RelationType::Founded)
        } else if INVESTED_VERBS.iter().any(|v| sentence_lower.contains(v)) {
            Some(RelationType::InvestedIn)
        } else if ADVISES_VERBS.iter().any(|v| sentence_lower.contains(v)) {
            Some(RelationType::Advises)
        } else if WORKS_AT_VERBS.iter().any(|v| sentence_lower.contains(v)) {
            Some(RelationType::WorksAt)
        } else if ATTENDED_VERBS.iter().any(|v| sentence_lower.contains(v)) {
            Some(RelationType::Attended)
        } else if MENTIONS_VERBS.iter().any(|v| sentence_lower.contains(v)) {
            Some(RelationType::Mentions)
        } else {
            None
        };

        // Create relations between co-occurring entities
        for i in 0..mentioned.len() {
            for j in i + 1..mentioned.len() {
                if mentioned[i].id != mentioned[j].id {
                    let rel_type = relation_type.unwrap_or(RelationType::RelatedTo);
                    let confidence = if relation_type.is_some() { 0.7 } else { 0.3 };

                    // Direction: person -> org/company
                    let (from, to) = if mentioned[i].entity_type == EntityType::Person
                        && (mentioned[j].entity_type == EntityType::Company
                            || mentioned[j].entity_type == EntityType::Organization)
                    {
                        (&mentioned[i].id, &mentioned[j].id)
                    } else if mentioned[j].entity_type == EntityType::Person
                        && (mentioned[i].entity_type == EntityType::Company
                            || mentioned[i].entity_type == EntityType::Organization)
                    {
                        (&mentioned[j].id, &mentioned[i].id)
                    } else {
                        // First entity is the subject
                        (&mentioned[i].id, &mentioned[j].id)
                    };

                    relations.push(Relation::new(
                        from.clone(),
                        to.clone(),
                        rel_type,
                        confidence,
                        provenance.to_string(),
                    ));
                }
            }
        }
    }

    // Deduplicate
    relations.sort_by(|a, b| {
        a.from_entity
            .0
            .cmp(&b.from_entity.0)
            .then_with(|| a.to_entity.0.cmp(&b.to_entity.0))
            .then_with(|| {
                relation_type_key(&a.relation_type).cmp(&relation_type_key(&b.relation_type))
            })
    });
    relations.dedup_by(|a, b| {
        a.from_entity == b.from_entity
            && a.to_entity == b.to_entity
            && a.relation_type == b.relation_type
    });

    relations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_entities_from_text() {
        let content = "Alice Chen works at Acme Corp. She founded the company aka 'The A-Team'.";
        let entities = extract_entities_from_content(content, "test-1");
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
        assert!(names
            .iter()
            .any(|n| n.to_lowercase().contains("alice chen")));
        assert!(names.iter().any(|n| n.to_lowercase().contains("acme corp")));
    }

    #[test]
    fn infer_works_at_relation() {
        let content = "Alice Chen works at Acme Corp as a senior engineer.";
        let entities = vec![
            Entity::new("Alice Chen".into(), EntityType::Person, "test".into()),
            Entity::new("Acme Corp".into(), EntityType::Company, "test".into()),
        ];
        let relations = infer_relations(content, &entities, "test");
        assert!(!relations.is_empty());
        assert_eq!(relations[0].relation_type, RelationType::WorksAt);
    }

    #[test]
    fn infer_founded_relation() {
        let content = "Alice Chen co-founded Acme Corp in 2020.";
        let entities = vec![
            Entity::new("Alice Chen".into(), EntityType::Person, "test".into()),
            Entity::new("Acme Corp".into(), EntityType::Company, "test".into()),
        ];
        let relations = infer_relations(content, &entities, "test");
        assert!(!relations.is_empty());
        assert_eq!(relations[0].relation_type, RelationType::Founded);
    }

    #[test]
    fn extract_markdown_links() {
        let content = "See [Alice Chen](people/alice) for details.";
        let entities = extract_entities_from_content(content, "test");
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Alice Chen"));
    }
}
