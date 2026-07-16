use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::RetentionRepository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionCompactionPolicy {
    pub min_tombstone_age_ms: i64,
    pub backup_completed_at_ms: Option<i64>,
    pub require_remote_settled: bool,
    pub batch_size: usize,
    pub lease_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionCompactionReport {
    pub removed: Vec<String>,
    pub watermark: Option<String>,
}

pub struct RetentionCompactor<'a> {
    repository: &'a RetentionRepository,
}

impl<'a> RetentionCompactor<'a> {
    pub fn new(repository: &'a RetentionRepository) -> Self {
        Self { repository }
    }

    pub fn run(
        &self,
        owner: &str,
        now_ms: i64,
        policy: &RetentionCompactionPolicy,
    ) -> anyhow::Result<RetentionCompactionReport> {
        anyhow::ensure!(
            !owner.trim().is_empty()
                && policy.lease_ms > 0
                && policy.batch_size > 0
                && policy.min_tombstone_age_ms >= 0,
            "invalid retention compaction policy"
        );
        let backup_at = policy.backup_completed_at_ms.ok_or_else(|| {
            anyhow::anyhow!("retention compaction requires a completed backup/checkpoint")
        })?;
        let mut connection = self.repository.connection();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let lease: Option<(Option<String>, Option<i64>)> = tx.query_row(
            "SELECT lease_owner,lease_until_ms FROM retention_compaction_state WHERE singleton=1", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        if let Some((Some(current_owner), Some(until))) = lease {
            anyhow::ensure!(
                current_owner == owner || until <= now_ms,
                "retention compaction lease is held"
            );
        }
        tx.execute(
            "UPDATE retention_compaction_state SET lease_owner=?1,lease_until_ms=?2 WHERE singleton=1",
            params![owner, now_ms.saturating_add(policy.lease_ms)],
        )?;
        let cutoff = now_ms.saturating_sub(policy.min_tombstone_age_ms);
        let remote_clause = if policy.require_remote_settled {
            "AND t.remote_state IN ('not_required','settled')"
        } else {
            ""
        };
        let sql = format!(
            "SELECT t.record_id,t.requested_ms FROM memory_tombstones t JOIN retention_records r USING(record_id)
             WHERE t.payload_removed_ms IS NULL AND r.record_json IS NOT NULL AND t.requested_ms<=?1 AND t.requested_ms<=?2 {remote_clause}
             ORDER BY t.requested_ms,t.record_id LIMIT ?3"
        );
        let rows = {
            let mut statement = tx.prepare(&sql)?;
            let rows = statement
                .query_map(
                    params![cutoff, backup_at, policy.batch_size as i64],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut report = RetentionCompactionReport::default();
        for (id, requested_ms) in rows {
            tx.execute(
                "UPDATE retention_records SET record_json=NULL WHERE record_id=?1",
                [&id],
            )?;
            tx.execute(
                "UPDATE memory_tombstones SET payload_removed_ms=?2 WHERE record_id=?1",
                params![id, now_ms],
            )?;
            report.watermark = Some(format!("{requested_ms}:{id}"));
            report.removed.push(id);
        }
        tx.execute(
            "UPDATE retention_compaction_state SET lease_owner=NULL,lease_until_ms=NULL,watermark=COALESCE(?1,watermark),last_compacted_ms=?2 WHERE singleton=1",
            params![report.watermark, now_ms],
        )?;
        tx.commit()?;
        Ok(report)
    }
}
