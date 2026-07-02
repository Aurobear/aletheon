use anyhow::Result;
use regex::Regex;

use super::{EntityNeighbor, FactRow, FactStore};

impl FactStore {
    // ── Entity Graph ─────────────────────────────────────────────────────────

    /// Extract entity names from content using regex patterns:
    /// capitalized multi-word phrases, double-quoted, single-quoted, and "X aka Y".
    pub fn extract_entities(content: &str) -> Vec<String> {
        let mut entities = Vec::new();

        // Capitalized multi-word: two or more consecutive capitalized words
        let cap_re = Regex::new(r"\b([A-Z][a-z]+(?:\s+[A-Z][a-z]+)+)\b").unwrap();
        for cap in cap_re.captures_iter(content) {
            entities.push(cap[1].to_string());
        }

        // Capitalized single words not mid-sentence (after . ! ?).
        // Matches any [A-Z][a-z]+ word, then we filter out those that appear
        // right after a sentence boundary (period/exclaim/question + space).
        let single_re = Regex::new(r"\b([A-Z][a-z]+)\b").unwrap();
        for m in single_re.find_iter(content) {
            let before = &content[..m.start()];
            let is_mid_sentence_start = before.ends_with(". ")
                || before.ends_with("! ")
                || before.ends_with("? ");
            if !is_mid_sentence_start {
                entities.push(m.as_str().to_string());
            }
        }

        // Double-quoted terms
        let dq_re = Regex::new(r#""([^"]{2,})""#).unwrap();
        for cap in dq_re.captures_iter(content) {
            entities.push(cap[1].to_string());
        }

        // Single-quoted terms
        let sq_re = Regex::new(r"'([^']{2,})'").unwrap();
        for cap in sq_re.captures_iter(content) {
            entities.push(cap[1].to_string());
        }

        // "X aka Y" pattern
        let aka_re = Regex::new(r"\b(\w[\w\s]+?)\s+aka\s+(\w[\w\s]+?)(?:\s|,|\.|$)").unwrap();
        for cap in aka_re.captures_iter(content) {
            let a = cap[1].trim().to_string();
            let b = cap[2].trim().to_string();
            if !a.is_empty() {
                entities.push(a);
            }
            if !b.is_empty() {
                entities.push(b);
            }
        }

        entities.sort();
        entities.dedup();
        entities
    }

    /// Resolve an entity name to its ID. Creates if not found.
    pub fn resolve_entity(&self, name: &str) -> Result<i64> {
        // Try exact match
        if let Ok(id) = self.db.query_row(
            "SELECT entity_id FROM entities WHERE name = ?1",
            rusqlite::params![name],
            |row| row.get::<_, i64>(0),
        ) {
            return Ok(id);
        }

        // Try alias match
        if let Ok(id) = self.db.query_row(
            "SELECT entity_id FROM entities WHERE ',' || aliases || ',' LIKE ?1",
            rusqlite::params![format!(",{},", name)],
            |row| row.get::<_, i64>(0),
        ) {
            return Ok(id);
        }

        // Create new entity
        self.db.execute(
            "INSERT INTO entities (name) VALUES (?1)",
            rusqlite::params![name],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Link a fact to an entity.
    pub fn link_fact_entity(&self, fact_id: i64, entity_id: i64) -> Result<()> {
        self.db.execute(
            "INSERT OR IGNORE INTO fact_entities (fact_id, entity_id) VALUES (?1, ?2)",
            rusqlite::params![fact_id, entity_id],
        )?;
        Ok(())
    }

    /// Get 1-hop neighbors of an entity via shared facts.
    pub fn get_entity_neighbors(&self, entity_id: i64) -> Result<Vec<EntityNeighbor>> {
        let mut stmt = self.db.prepare(
            "SELECT e.entity_id, e.name, COUNT(DISTINCT fe2.fact_id) as shared_facts
             FROM fact_entities fe1
             JOIN fact_entities fe2 ON fe1.fact_id = fe2.fact_id AND fe2.entity_id != ?1
             JOIN entities e ON fe2.entity_id = e.entity_id
             WHERE fe1.entity_id = ?1
             GROUP BY e.entity_id, e.name
             ORDER BY shared_facts DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![entity_id], |row| {
            Ok(EntityNeighbor {
                entity_id: row.get(0)?,
                name: row.get(1)?,
                shared_facts: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// BFS path between two entities, up to max_depth hops.
    pub fn find_entity_path(
        &self,
        from_id: i64,
        to_id: i64,
        max_depth: usize,
    ) -> Result<Option<Vec<i64>>> {
        use std::collections::{HashSet, VecDeque};

        if from_id == to_id {
            return Ok(Some(vec![from_id]));
        }

        let mut visited = HashSet::new();
        visited.insert(from_id);
        let mut queue = VecDeque::new();
        queue.push_back(vec![from_id]);

        while let Some(path) = queue.pop_front() {
            if path.len() > max_depth {
                continue;
            }
            let current = *path.last().unwrap();
            let neighbors = self.get_entity_neighbors(current)?;
            for nb in neighbors {
                if nb.entity_id == to_id {
                    let mut result = path;
                    result.push(to_id);
                    return Ok(Some(result));
                }
                if !visited.contains(&nb.entity_id) {
                    visited.insert(nb.entity_id);
                    let mut new_path = path.clone();
                    new_path.push(nb.entity_id);
                    queue.push_back(new_path);
                }
            }
        }
        Ok(None)
    }

    /// Get all facts linked to an entity.
    pub fn get_entity_facts(&self, entity_id: i64) -> Result<Vec<FactRow>> {
        let mut stmt = self.db.prepare(
            "SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
                    f.trust_score, f.retrieval_count, f.helpful_count,
                    f.tier, f.ttl_days, f.created_at, f.updated_at,
                    f.scope, f.source, f.status, f.pinned, f.subject
             FROM facts f
             JOIN fact_entities fe ON f.fact_id = fe.fact_id
             WHERE fe.entity_id = ?1
             ORDER BY f.trust_score DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![entity_id], Self::map_fact_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}
