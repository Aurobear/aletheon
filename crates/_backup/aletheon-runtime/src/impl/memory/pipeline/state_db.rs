use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Status of a session's Phase 1 processing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage1Status {
    /// Waiting to be picked up.
    Pending,
    /// Currently being processed by a worker.
    Claimed { claim_id: String },
    /// Extraction completed successfully.
    Succeeded,
    /// Extraction failed.
    Failed { reason: String },
}

/// Record of a single session tracked by the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub session_path: PathBuf,
    pub stage1_status: Stage1Status,
    pub raw_memory: Option<String>,
    pub summary: Option<String>,
    pub slug: Option<String>,
    pub usage_count: u32,
    pub last_used: u64,
    pub created_at: u64,
}

impl SessionRecord {
    pub fn new(session_id: String, session_path: PathBuf, now: u64) -> Self {
        Self {
            session_id,
            session_path,
            stage1_status: Stage1Status::Pending,
            raw_memory: None,
            summary: None,
            slug: None,
            usage_count: 0,
            last_used: now,
            created_at: now,
        }
    }
}

/// In-memory state database for tracking session processing status.
///
/// No external DB dependency — all state lives in memory and is
/// reconstructed from on-disk session files on startup.
pub struct StateDatabase {
    sessions: HashMap<String, SessionRecord>,
    phase2_lock: Option<String>,
}

impl StateDatabase {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            phase2_lock: None,
        }
    }

    /// Insert or replace a session record.
    pub fn upsert_session(&mut self, record: SessionRecord) {
        debug!(session_id = %record.session_id, "Upserting session record");
        self.sessions.insert(record.session_id.clone(), record);
    }

    /// Get a reference to a session record.
    pub fn get_session(&self, session_id: &str) -> Option<&SessionRecord> {
        self.sessions.get(session_id)
    }

    /// Get a mutable reference to a session record.
    pub fn get_session_mut(&mut self, session_id: &str) -> Option<&mut SessionRecord> {
        self.sessions.get_mut(session_id)
    }

    /// Total number of tracked sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Claim up to `limit` eligible sessions for Phase 1 processing.
    ///
    /// Eligible = Pending and created_at >= now - max_age_secs and
    /// idle for at least min_idle_secs.
    pub fn claim_sessions(
        &mut self,
        limit: usize,
        now: u64,
        max_age_secs: u64,
        min_idle_secs: u64,
        claim_id: &str,
    ) -> Vec<String> {
        let mut claimed = Vec::new();

        for record in self.sessions.values_mut() {
            if claimed.len() >= limit {
                break;
            }

            // Only claim Pending sessions.
            if record.stage1_status != Stage1Status::Pending {
                continue;
            }

            // Check max age.
            if now.saturating_sub(record.created_at) > max_age_secs {
                continue;
            }

            // Check minimum idle time.
            if now.saturating_sub(record.last_used) < min_idle_secs {
                continue;
            }

            record.stage1_status = Stage1Status::Claimed {
                claim_id: claim_id.to_string(),
            };
            claimed.push(record.session_id.clone());
            debug!(session_id = %record.session_id, claim_id, "Session claimed for Phase 1");
        }

        claimed
    }

    /// Mark a session as succeeded with its extraction results.
    pub fn mark_succeeded(
        &mut self,
        session_id: &str,
        raw_memory: String,
        summary: String,
        slug: String,
    ) -> anyhow::Result<()> {
        let record = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_id))?;

        record.stage1_status = Stage1Status::Succeeded;
        record.raw_memory = Some(raw_memory);
        record.summary = Some(summary);
        record.slug = Some(slug);
        debug!(session_id, "Session marked as succeeded");
        Ok(())
    }

    /// Mark a session as failed.
    pub fn mark_failed(&mut self, session_id: &str, reason: String) -> anyhow::Result<()> {
        let record = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_id))?;

        record.stage1_status = Stage1Status::Failed { reason: reason.clone() };
        warn!(session_id, reason, "Session marked as failed");
        Ok(())
    }

    /// Release a claimed session back to Pending (e.g. on timeout).
    pub fn release_claim(&mut self, session_id: &str) -> anyhow::Result<()> {
        let record = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_id))?;

        if let Stage1Status::Claimed { .. } = record.stage1_status {
            record.stage1_status = Stage1Status::Pending;
            debug!(session_id, "Session claim released");
            Ok(())
        } else {
            anyhow::bail!("Session '{}' is not in Claimed state", session_id)
        }
    }

    /// Get all succeeded sessions sorted by (usage_count desc, last_used desc).
    pub fn succeeded_sessions(&self) -> Vec<&SessionRecord> {
        let mut records: Vec<_> = self
            .sessions
            .values()
            .filter(|r| r.stage1_status == Stage1Status::Succeeded)
            .collect();
        records.sort_by(|a, b| {
            b.usage_count
                .cmp(&a.usage_count)
                .then(b.last_used.cmp(&a.last_used))
        });
        records
    }

    /// Get all sessions with a given status.
    pub fn sessions_by_status(&self, status: &Stage1Status) -> Vec<&SessionRecord> {
        self.sessions
            .values()
            .filter(|r| &r.stage1_status == status)
            .collect()
    }

    // --- Phase 2 lock management ---

    /// Try to acquire the Phase 2 global lock. Returns true if acquired.
    pub fn acquire_phase2_lock(&mut self, claim_id: &str) -> bool {
        if self.phase2_lock.is_some() {
            return false;
        }
        self.phase2_lock = Some(claim_id.to_string());
        debug!(claim_id, "Phase 2 lock acquired");
        true
    }

    /// Release the Phase 2 global lock.
    pub fn release_phase2_lock(&mut self, claim_id: &str) -> anyhow::Result<()> {
        match &self.phase2_lock {
            Some(lock_id) if lock_id == claim_id => {
                self.phase2_lock = None;
                debug!(claim_id, "Phase 2 lock released");
                Ok(())
            }
            Some(other) => {
                anyhow::bail!(
                    "Phase 2 lock is held by '{}', not '{}'",
                    other,
                    claim_id
                )
            }
            None => anyhow::bail!("No Phase 2 lock is currently held"),
        }
    }

    /// Check if Phase 2 lock is held.
    pub fn is_phase2_locked(&self) -> bool {
        self.phase2_lock.is_some()
    }

    /// Get the current Phase 2 lock holder's claim_id.
    pub fn phase2_lock_holder(&self) -> Option<&str> {
        self.phase2_lock.as_deref()
    }
}

impl Default for StateDatabase {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(secs: u64) -> u64 {
        secs
    }

    #[test]
    fn test_session_record_lifecycle() {
        let record = SessionRecord::new(
            "sess-1".into(),
            PathBuf::from("/tmp/sessions/sess-1"),
            ts(1000),
        );
        assert_eq!(record.session_id, "sess-1");
        assert_eq!(record.stage1_status, Stage1Status::Pending);
        assert!(record.raw_memory.is_none());
        assert_eq!(record.usage_count, 0);
        assert_eq!(record.created_at, 1000);
    }

    #[test]
    fn test_upsert_and_get() {
        let mut db = StateDatabase::new();
        let record = SessionRecord::new(
            "s1".into(),
            PathBuf::from("/tmp/s1"),
            ts(100),
        );
        db.upsert_session(record);

        assert_eq!(db.session_count(), 1);
        let got = db.get_session("s1").unwrap();
        assert_eq!(got.session_id, "s1");
        assert!(db.get_session("nonexistent").is_none());
    }

    #[test]
    fn test_claim_sessions_respects_limits() {
        let mut db = StateDatabase::new();

        // Insert 5 sessions, all idle for 100s, created at t=0.
        for i in 0..5 {
            let mut r = SessionRecord::new(
                format!("s{}", i),
                PathBuf::from(format!("/tmp/s{}", i)),
                ts(0),
            );
            r.last_used = ts(0); // idle since t=0
            db.upsert_session(r);
        }

        // At t=200, claim up to 3 with min_idle=50, max_age=1000.
        let claimed = db.claim_sessions(3, ts(200), 1000, 50, "claim-1");
        assert_eq!(claimed.len(), 3);

        // Verify they are Claimed.
        for sid in &claimed {
            let rec = db.get_session(sid).unwrap();
            match &rec.stage1_status {
                Stage1Status::Claimed { claim_id } => assert_eq!(claim_id, "claim-1"),
                other => panic!("Expected Claimed, got {:?}", other),
            }
        }

        // Claiming again should get the remaining 2.
        let claimed2 = db.claim_sessions(3, ts(200), 1000, 50, "claim-2");
        assert_eq!(claimed2.len(), 2);
    }

    #[test]
    fn test_claim_sessions_skips_too_young() {
        let mut db = StateDatabase::new();
        let mut r = SessionRecord::new("s1".into(), PathBuf::from("/tmp/s1"), ts(0));
        r.last_used = ts(0);
        db.upsert_session(r);

        // Session is idle for 10s but min_idle is 50s — should not be claimed.
        let claimed = db.claim_sessions(10, ts(10), 1000, 50, "c1");
        assert!(claimed.is_empty());
    }

    #[test]
    fn test_claim_sessions_skips_expired() {
        let mut db = StateDatabase::new();
        let mut r = SessionRecord::new("s1".into(), PathBuf::from("/tmp/s1"), ts(0));
        r.last_used = ts(0);
        db.upsert_session(r);

        // Created at t=0, now=2000, max_age=1000 — expired.
        let claimed = db.claim_sessions(10, ts(2000), 1000, 0, "c1");
        assert!(claimed.is_empty());
    }

    #[test]
    fn test_mark_succeeded_and_failed() {
        let mut db = StateDatabase::new();
        let r = SessionRecord::new("s1".into(), PathBuf::from("/tmp/s1"), ts(0));
        db.upsert_session(r);

        db.mark_succeeded(
            "s1",
            "raw content".into(),
            "summary text".into(),
            "my-slug".into(),
        )
        .unwrap();

        let rec = db.get_session("s1").unwrap();
        assert_eq!(rec.stage1_status, Stage1Status::Succeeded);
        assert_eq!(rec.raw_memory.as_deref(), Some("raw content"));
        assert_eq!(rec.summary.as_deref(), Some("summary text"));
        assert_eq!(rec.slug.as_deref(), Some("my-slug"));

        // Mark another as failed.
        let r2 = SessionRecord::new("s2".into(), PathBuf::from("/tmp/s2"), ts(0));
        db.upsert_session(r2);
        db.mark_failed("s2", "parse error".into()).unwrap();

        let rec2 = db.get_session("s2").unwrap();
        match &rec2.stage1_status {
            Stage1Status::Failed { reason } => assert_eq!(reason, "parse error"),
            other => panic!("Expected Failed, got {:?}", other),
        }
    }

    #[test]
    fn test_release_claim() {
        let mut db = StateDatabase::new();
        let mut r = SessionRecord::new("s1".into(), PathBuf::from("/tmp/s1"), ts(0));
        r.last_used = ts(0);
        db.upsert_session(r);

        db.claim_sessions(1, ts(100), 1000, 0, "c1");
        assert!(matches!(
            db.get_session("s1").unwrap().stage1_status,
            Stage1Status::Claimed { .. }
        ));

        db.release_claim("s1").unwrap();
        assert_eq!(
            db.get_session("s1").unwrap().stage1_status,
            Stage1Status::Pending
        );
    }

    #[test]
    fn test_release_claim_wrong_state() {
        let mut db = StateDatabase::new();
        let r = SessionRecord::new("s1".into(), PathBuf::from("/tmp/s1"), ts(0));
        db.upsert_session(r);

        // Pending is not Claimed — should error.
        assert!(db.release_claim("s1").is_err());
    }

    #[test]
    fn test_succeeded_sessions_sorted() {
        let mut db = StateDatabase::new();

        let mut r1 = SessionRecord::new("s1".into(), PathBuf::from("/tmp/s1"), ts(0));
        r1.stage1_status = Stage1Status::Succeeded;
        r1.usage_count = 5;
        r1.last_used = ts(100);

        let mut r2 = SessionRecord::new("s2".into(), PathBuf::from("/tmp/s2"), ts(0));
        r2.stage1_status = Stage1Status::Succeeded;
        r2.usage_count = 10;
        r2.last_used = ts(50);

        let mut r3 = SessionRecord::new("s3".into(), PathBuf::from("/tmp/s3"), ts(0));
        r3.stage1_status = Stage1Status::Succeeded;
        r3.usage_count = 10;
        r3.last_used = ts(200);

        db.upsert_session(r1);
        db.upsert_session(r2);
        db.upsert_session(r3);

        let succeeded = db.succeeded_sessions();
        assert_eq!(succeeded.len(), 3);
        // s3 (usage=10, last_used=200) > s2 (usage=10, last_used=50) > s1 (usage=5)
        assert_eq!(succeeded[0].session_id, "s3");
        assert_eq!(succeeded[1].session_id, "s2");
        assert_eq!(succeeded[2].session_id, "s1");
    }

    #[test]
    fn test_phase2_lock() {
        let mut db = StateDatabase::new();

        assert!(!db.is_phase2_locked());
        assert!(db.acquire_phase2_lock("claim-a"));
        assert!(db.is_phase2_locked());
        assert_eq!(db.phase2_lock_holder(), Some("claim-a"));

        // Cannot acquire while locked.
        assert!(!db.acquire_phase2_lock("claim-b"));

        // Wrong claim_id cannot release.
        assert!(db.release_phase2_lock("claim-b").is_err());

        // Correct claim_id releases.
        db.release_phase2_lock("claim-a").unwrap();
        assert!(!db.is_phase2_locked());

        // Now claim-b can acquire.
        assert!(db.acquire_phase2_lock("claim-b"));
    }

    #[test]
    fn test_sessions_by_status() {
        let mut db = StateDatabase::new();

        for i in 0..3 {
            let mut r = SessionRecord::new(
                format!("s{}", i),
                PathBuf::from(format!("/tmp/s{}", i)),
                ts(0),
            );
            if i == 0 {
                r.stage1_status = Stage1Status::Succeeded;
            }
            db.upsert_session(r);
        }

        assert_eq!(
            db.sessions_by_status(&Stage1Status::Pending).len(),
            2
        );
        assert_eq!(
            db.sessions_by_status(&Stage1Status::Succeeded).len(),
            1
        );
    }
}
