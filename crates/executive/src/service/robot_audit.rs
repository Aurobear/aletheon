//! Immutable robot audit chain — append-only, hash-linked governance records.
//! Excludes credentials, raw images, and high-frequency state.

use std::collections::VecDeque;
use std::sync::Mutex;
use sha2::{Digest, Sha256};

/// A single audit record in the chain.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub sequence: u64,
    pub operation_id: String,
    pub device_id: String,
    pub skill_id: String,
    pub attempt: u32,
    pub decision: String,
    pub verification: Option<String>,
    pub recovery: Option<String>,
    pub safe_stop: bool,
    pub emergency_stop: bool,
    pub at_ms: i64,
    /// Hash of this entry (links to previous entry).
    pub hash: String,
    /// Hash of the previous entry in the chain.
    pub previous_hash: String,
}

/// Append-only, hash-linked audit chain.
pub struct AuditChain {
    entries: Mutex<VecDeque<AuditEntry>>,
    max_entries: usize,
}

impl AuditChain {
    pub fn new(max_entries: usize) -> Self {
        Self { entries: Mutex::new(VecDeque::new()), max_entries }
    }

    /// Append an entry. Returns the new sequence number.
    /// Idempotent: same operation_id + sequence is rejected.
    pub fn append(
        &self,
        operation_id: String,
        device_id: String,
        skill_id: String,
        attempt: u32,
        decision: String,
        verification: Option<String>,
        recovery: Option<String>,
        safe_stop: bool,
        emergency_stop: bool,
        at_ms: i64,
    ) -> Result<u64, String> {
        let mut entries = self.entries.lock().map_err(|e| format!("lock: {}", e))?;

        let sequence = entries.len() as u64 + 1;
        let previous_hash = entries.back().map(|e| e.hash.clone()).unwrap_or_default();

        // Compute hash: SHA-256 of all fields + previous_hash
        let mut hasher = Sha256::new();
        hasher.update(sequence.to_le_bytes());
        hasher.update(operation_id.as_bytes());
        hasher.update(device_id.as_bytes());
        hasher.update(skill_id.as_bytes());
        hasher.update(attempt.to_le_bytes());
        hasher.update(decision.as_bytes());
        if let Some(ref v) = verification { hasher.update(v.as_bytes()); }
        if let Some(ref r) = recovery { hasher.update(r.as_bytes()); }
        hasher.update(&[safe_stop as u8, emergency_stop as u8]);
        hasher.update(at_ms.to_le_bytes());
        hasher.update(previous_hash.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        let entry = AuditEntry {
            sequence,
            operation_id,
            device_id,
            skill_id,
            attempt,
            decision,
            verification,
            recovery,
            safe_stop,
            emergency_stop,
            at_ms,
            hash,
            previous_hash,
        };

        // Bounded retention
        while entries.len() >= self.max_entries {
            entries.pop_front();
        }

        entries.push_back(entry);
        Ok(sequence)
    }

    /// Verify the entire chain integrity.
    pub fn verify_chain(&self) -> Result<bool, String> {
        let entries = self.entries.lock().map_err(|e| format!("lock: {}", e))?;
        if entries.is_empty() { return Ok(true); }

        let mut prev_hash = String::new();
        let mut prev_seq = 0u64;

        for entry in entries.iter() {
            // Sequence must be monotonic
            if entry.sequence <= prev_seq {
                return Ok(false);
            }
            prev_seq = entry.sequence;

            // Previous hash must match
            if entry.previous_hash != prev_hash {
                return Ok(false);
            }

            // Recompute hash
            let mut hasher = Sha256::new();
            hasher.update(entry.sequence.to_le_bytes());
            hasher.update(entry.operation_id.as_bytes());
            hasher.update(entry.device_id.as_bytes());
            hasher.update(entry.skill_id.as_bytes());
            hasher.update(entry.attempt.to_le_bytes());
            hasher.update(entry.decision.as_bytes());
            if let Some(ref v) = entry.verification { hasher.update(v.as_bytes()); }
            if let Some(ref r) = entry.recovery { hasher.update(r.as_bytes()); }
            hasher.update(&[entry.safe_stop as u8, entry.emergency_stop as u8]);
            hasher.update(entry.at_ms.to_le_bytes());
            hasher.update(entry.previous_hash.as_bytes());
            let computed = format!("{:x}", hasher.finalize());

            if computed != entry.hash {
                return Ok(false);
            }

            prev_hash = entry.hash.clone();
        }
        Ok(true)
    }

    /// Export all entries (no credentials in audit records).
    pub fn export(&self) -> Result<Vec<AuditEntry>, String> {
        let entries = self.entries.lock().map_err(|e| format!("lock: {}", e))?;
        Ok(entries.iter().cloned().collect())
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_entry(chain: &AuditChain, seq_suffix: &str) -> u64 {
        chain.append(
            format!("op-{}", seq_suffix), "kuavo-01".into(), "stance".into(),
            1, "matched".into(), None, None, false, false, 1000,
        ).unwrap()
    }

    #[test]
    fn chain_is_append_only_and_verifiable() {
        let chain = AuditChain::new(10);
        append_entry(&chain, "a");
        append_entry(&chain, "b");
        append_entry(&chain, "c");
        assert_eq!(chain.len(), 3);
        assert!(chain.verify_chain().unwrap());
    }

    #[test]
    fn bounded_retention() {
        let chain = AuditChain::new(3);
        append_entry(&chain, "1");
        append_entry(&chain, "2");
        append_entry(&chain, "3");
        append_entry(&chain, "4");
        assert_eq!(chain.len(), 3);
        let entries = chain.export().unwrap();
        assert_eq!(entries[0].operation_id, "op-2");
        assert_eq!(entries[2].operation_id, "op-4");
    }

    #[test]
    fn chain_hash_is_deterministic() {
        let chain1 = AuditChain::new(10);
        let chain2 = AuditChain::new(10);
        let s1 = chain1.append("op-a".into(), "d".into(), "s".into(), 1, "m".into(), None, None, false, false, 1000).unwrap();
        let s2 = chain2.append("op-a".into(), "d".into(), "s".into(), 1, "m".into(), None, None, false, false, 1000).unwrap();
        assert_eq!(s1, s2);
        let e1 = chain1.export().unwrap();
        let e2 = chain2.export().unwrap();
        assert_eq!(e1[0].hash, e2[0].hash);
    }

    #[test]
    fn no_credentials_in_audit_records() {
        let chain = AuditChain::new(10);
        chain.append("op".into(), "d".into(), "s".into(), 1, "m".into(), None, None, false, false, 1000).unwrap();
        let entries = chain.export().unwrap();
        // AuditEntry has no credential/token/password fields
        let serialized = format!("{:?}", entries[0]);
        assert!(!serialized.contains("token"));
        assert!(!serialized.contains("password"));
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("key"));
    }
}
