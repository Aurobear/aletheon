# Production operations checklist

## Install or release

- [ ] Verify supported Ubuntu/Pi architecture, time sync, disk/inode headroom.
- [ ] Verify binary, assets, configuration, service units, and image digests.
- [ ] Initialize/audit credentials; keep recovery keys separately encrypted.
- [ ] Run systemd preflight, unit verification, and network exposure verification.
- [ ] Confirm release candidate, `/usr/bin/aletheon`, and both running daemon
      executables have the same SHA-256 digest.
- [ ] Observe unchanged machine/user daemon PIDs and restart counters for the
      configured stability window.
- [ ] Scan current-PID journals for profile/tool validation failures, panics,
      main-process exits, and failed unit results.
- [ ] Complete a real LLM request using `/usr/bin/aletheon` and the official
      per-user socket; do not substitute a debug or isolated runtime.
- [ ] Enable daemon, daily backup and cleanup timers; confirm next calendar times.
- [ ] Wait for readiness and test Telegram, Google, and GBrain only when enabled.
- [ ] Create a test Goal and verify attempt, approval, audit, and restart recovery.
- [ ] Record boot/restart duration, backup size/time, disk headroom, and receipts.

## Daily and weekly

- [ ] Daily: readiness class, failed units, queue/retry depth, sync lag, free bytes/inodes.
- [ ] Daily: latest encrypted backup age and cleanup outcome.
- [ ] Weekly: Restic check/prune/remote replication and a sampled data read.
- [ ] Weekly: audit continuity/rotation, worker crashes, oldest approval, dead letters.
- [ ] Monthly: tailnet devices/ACLs, credential age, capacity trend, recovery contacts.

## Release acceptance

- [ ] Run `bash scripts/aletheon.sh deploy` and retain its provenance, stability,
      real-request, and final-health evidence.
- [ ] Keep test-only profiles under isolated state roots; verify no unmatched
      profile was copied into active user state.
- [ ] Restore a live-WAL/active-Goal snapshot on an empty disposable host.
- [ ] Exercise upgrade, failed readiness, matching-data rollback, and old-root recovery.
- [ ] Interrupt Google/GBrain transactions and prove cursor/spool idempotence.
- [ ] Fill every quota and prove protected evidence survives cleanup.
- [ ] Reboot and inject worker/container/network/secret/store failures.
- [ ] Verify approved Tailscale access and denied LAN/unapproved/public access.
- [ ] Run canary audit across Git, argv/env, journal, audit, backups, and transcripts.
- [ ] Record measured RPO/RTO and attach redacted command transcripts.

If a required check is unavailable or ambiguous, do not call the release ready;
record the missing evidence, owner, and remediation date.
