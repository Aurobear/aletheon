# Managed storage policy

Production limits are configured under `[deployment.quotas]` for artifacts,
worktrees, audit, sessions, Google projection/content, GBrain spool/dead letters,
and the total data root. Each class needs a soft alert threshold plus a hard
byte/item admission limit. Keep at least 5 GiB and 10% of the filesystem free;
alert on inode use above 85%. Use filesystem/project quotas as a second boundary
where the host supports them.

Artifact writes reserve their maximum expected bytes before creating a temporary
file. The shared quota primitive counts existing regular files plus concurrent
reservations and rejects symlinks and multiply-linked files. Worktrees already
enforce `disk_budget_bytes`; Google event/projection/cursor payloads and GBrain
spool items/bytes have transactional caps. A hard-limit error is explicit and
must block a new Goal attempt, download, artifact, or projection rather than
deleting evidence. A dropped in-process reservation releases automatically;
after a crash, no reservation ledger can leak and actual filesystem usage is
rescanned.

Cleanup is fail-closed. A producer may add `.cleanup-after` with an expiry epoch
only after the entry is acknowledged or a worktree is verified clean. The daily
job processes abandoned clean worktrees, caches, expired sessions, then retained
artifacts. It never removes an entry containing `.active`, `.pinned`, or
`.legal-hold`, and it rejects symlinks or paths outside the managed root. Google
outbox/projection and GBrain dead-letter deletion must use their transactional
acknowledgement APIs rather than filesystem cleanup.

Always preserve active Goal/attempt and approval evidence, the latest successful
backup receipt, minimum audit retention (365 days unless policy requires more),
and legal/user pins. Low bytes or inodes changes deployment health to degraded;
crossing a required hard boundary changes readiness to unready until cleanup or
capacity expansion succeeds.
