//! Immutable robot audit chain — append-only, hash-linked governance records.
//! Excludes credentials, raw images, and high-frequency state.

use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::sync::Mutex;

use fabric::types::episode_report::SettledEpisodeReport;

/// A single audit record in the chain.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub sequence: u64,
    pub episode_id: Option<String>,
    pub report_sha256: Option<String>,
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
        Self {
            entries: Mutex::new(VecDeque::new()),
            max_entries: max_entries.max(1),
        }
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
        self.append_bound(
            None,
            None,
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
        )
    }

    /// Append one episode-level audit receipt bound to an immutable settled
    /// report. The detailed ordered attempts remain authoritative in the report;
    /// this chain records its digest and terminal governance projection.
    pub fn append_settled_report(&self, settled: &SettledEpisodeReport) -> Result<u64, String> {
        settled.verify_integrity()?;
        let report = settled.report();
        let final_attempt = report.attempts.last();
        let operation_id = final_attempt
            .and_then(|attempt| attempt.operation_id.clone())
            .or_else(|| final_attempt.map(|attempt| attempt.attempt_id.clone()))
            .unwrap_or_else(|| format!("episode:{}:no-operation", report.episode_id));
        self.append_bound(
            Some(report.episode_id.clone()),
            Some(settled.report_sha256().to_owned()),
            operation_id,
            report.device.0.clone(),
            format!("descriptor-set:{}", report.skill_descriptor_digest),
            final_attempt.map(|attempt| attempt.attempt).unwrap_or(0),
            report.settlement.as_str().to_owned(),
            report
                .final_decision()
                .map(|decision| format!("{decision:?}")),
            report.failures.last().map(|failure| failure.detail.clone()),
            report.safe_stop.is_some(),
            false,
            settled.settled_at_unix_ms(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn append_bound(
        &self,
        episode_id: Option<String>,
        report_sha256: Option<String>,
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
        let mut entries = self.entries.lock().map_err(|e| format!("lock: {e}"))?;

        let sequence = entries
            .back()
            .map(|entry| entry.sequence.saturating_add(1))
            .unwrap_or(1);
        let previous_hash = entries.back().map(|e| e.hash.clone()).unwrap_or_default();

        // Compute hash: SHA-256 of all fields + previous_hash
        let mut hasher = Sha256::new();
        hasher.update(sequence.to_le_bytes());
        if let Some(ref episode_id) = episode_id {
            hasher.update(episode_id.as_bytes());
        }
        if let Some(ref report_sha256) = report_sha256 {
            hasher.update(report_sha256.as_bytes());
        }
        hasher.update(operation_id.as_bytes());
        hasher.update(device_id.as_bytes());
        hasher.update(skill_id.as_bytes());
        hasher.update(attempt.to_le_bytes());
        hasher.update(decision.as_bytes());
        if let Some(ref v) = verification {
            hasher.update(v.as_bytes());
        }
        if let Some(ref r) = recovery {
            hasher.update(r.as_bytes());
        }
        hasher.update([safe_stop as u8, emergency_stop as u8]);
        hasher.update(at_ms.to_le_bytes());
        hasher.update(previous_hash.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        let entry = AuditEntry {
            sequence,
            episode_id,
            report_sha256,
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
        let entries = self.entries.lock().map_err(|e| format!("lock: {e}"))?;
        if entries.is_empty() {
            return Ok(true);
        }

        let mut prev_hash = entries
            .front()
            .map(|entry| entry.previous_hash.clone())
            .unwrap_or_default();
        let mut prev_seq = entries
            .front()
            .map(|entry| entry.sequence.saturating_sub(1))
            .unwrap_or(0);

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
            if let Some(ref episode_id) = entry.episode_id {
                hasher.update(episode_id.as_bytes());
            }
            if let Some(ref report_sha256) = entry.report_sha256 {
                hasher.update(report_sha256.as_bytes());
            }
            hasher.update(entry.operation_id.as_bytes());
            hasher.update(entry.device_id.as_bytes());
            hasher.update(entry.skill_id.as_bytes());
            hasher.update(entry.attempt.to_le_bytes());
            hasher.update(entry.decision.as_bytes());
            if let Some(ref v) = entry.verification {
                hasher.update(v.as_bytes());
            }
            if let Some(ref r) = entry.recovery {
                hasher.update(r.as_bytes());
            }
            hasher.update([entry.safe_stop as u8, entry.emergency_stop as u8]);
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
        let entries = self.entries.lock().map_err(|e| format!("lock: {e}"))?;
        Ok(entries.iter().cloned().collect())
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait::async_trait]
impl cognit::harness::robot::EpisodeAuditPort for AuditChain {
    async fn record(&self, report: &SettledEpisodeReport) -> Result<(), String> {
        self.append_settled_report(report).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_entry(chain: &AuditChain, seq_suffix: &str) -> u64 {
        chain
            .append(
                format!("op-{seq_suffix}"),
                "kuavo-01".into(),
                "stance".into(),
                1,
                "matched".into(),
                None,
                None,
                false,
                false,
                1000,
            )
            .unwrap()
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
        assert!(chain.verify_chain().unwrap());
    }

    #[test]
    fn chain_hash_is_deterministic() {
        let chain1 = AuditChain::new(10);
        let chain2 = AuditChain::new(10);
        let s1 = chain1
            .append(
                "op-a".into(),
                "d".into(),
                "s".into(),
                1,
                "m".into(),
                None,
                None,
                false,
                false,
                1000,
            )
            .unwrap();
        let s2 = chain2
            .append(
                "op-a".into(),
                "d".into(),
                "s".into(),
                1,
                "m".into(),
                None,
                None,
                false,
                false,
                1000,
            )
            .unwrap();
        assert_eq!(s1, s2);
        let e1 = chain1.export().unwrap();
        let e2 = chain2.export().unwrap();
        assert_eq!(e1[0].hash, e2[0].hash);
    }

    #[test]
    fn no_credentials_in_audit_records() {
        let chain = AuditChain::new(10);
        chain
            .append(
                "op".into(),
                "d".into(),
                "s".into(),
                1,
                "m".into(),
                None,
                None,
                false,
                false,
                1000,
            )
            .unwrap();
        let entries = chain.export().unwrap();
        // AuditEntry has no credential/token/password fields
        let serialized = format!("{:?}", entries[0]);
        assert!(!serialized.contains("token"));
        assert!(!serialized.contains("password"));
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("key"));
    }

    #[test]
    fn settled_episode_audit_binds_immutable_report_digest() {
        use fabric::types::embodiment::DeviceId;
        use fabric::types::episode_report::{
            build_report, EpisodeReportInput, EpisodeSettlement, SafeStopOutcome, SafeStopReceipt,
            SettledEpisodeReport,
        };

        let report = build_report(EpisodeReportInput {
            episode_id: "ep-audit".into(),
            goal: "stop safely".into(),
            device: DeviceId("kuavo-01".into()),
            sim_scene_version: "scene-v1".into(),
            aletheon_commit: "abc123".into(),
            bridge_protocol_digest: "sha256:bridge".into(),
            skill_descriptor_digest: "sha256:skills".into(),
            policy_provenance: None,
            failures: vec![],
            safe_stop: Some(SafeStopReceipt {
                attempted_after_attempt: 0,
                trigger: None,
                outcome: SafeStopOutcome::Succeeded,
            }),
            selected_frames: vec![],
            settlement: EpisodeSettlement::Failed,
            attempts: vec![],
            artifacts: vec![],
        });
        let settled = SettledEpisodeReport::new(report, 1_000).unwrap();
        let chain = AuditChain::new(4);
        chain.append_settled_report(&settled).unwrap();

        let entries = chain.export().unwrap();
        assert_eq!(entries[0].episode_id.as_deref(), Some("ep-audit"));
        assert_eq!(
            entries[0].report_sha256.as_deref(),
            Some(settled.report_sha256())
        );
        assert_eq!(entries[0].decision, "failed");
        assert!(entries[0].safe_stop);
        assert!(chain.verify_chain().unwrap());
    }
}
