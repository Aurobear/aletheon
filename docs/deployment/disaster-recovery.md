# Disaster recovery

Targets are **RPO 24 hours** for local state and **RTO 4 hours** on a replacement
host. Google/provider data may be replayed from the provider, but Goal,
approval, audit, and local-memory state use the stricter target. These are
planning bounds until measured in the release drill; record actual values.

| Incident | Contain | Recover | Escalate/notify |
|---|---|---|---|
| Host loss or SSD failure | Revoke Tailscale node and stop writes | Provision supported host, restore latest checked snapshot and separate keys | Owner immediately; provider owners if RPO exceeded |
| Corrupt Goal DB | Stop daemon; preserve DB/WAL read-only | Try offline integrity/export on a copy, otherwise restore matching snapshot | Owner; mark Goals since RPO for reconstruction |
| Lost Telegram token | Disable channel | Regenerate, atomically rotate, restart and test owner binding | Owner; revoke old token first if exposed |
| Lost Google credential | Disable Google integration | Reauthorize owner and resume from durable cursor | Owner; review provider security events |
| Compromised vault key | Stop Google and preserve evidence | Revoke OAuth grants, generate new key, reauthorize; treat backups as exposed | Owner/security immediately |
| GBrain loss | Keep local Mnemosyne authoritative; stop projection worker | Restore/export or rebuild optional container, replay durable spool | Degraded notification; no Goal-state rollback |
| Stuck worker | Cancel intake and TERM worker; preserve worktree | Restart daemon, reclaim expired lease, verify worktree before cleanup | Owner if retry budget exhausted |
| Full disk/inodes | Stop new attempts/downloads; never delete active evidence | Run managed cleanup, expand quota/filesystem, verify DBs and readiness | Urgent owner alert |
| Tailscale loss | Keep daemon local and public firewall closed | Use console recovery, re-enrol node, verify ACL/firewall | Tailnet admin; never open temporary WAN SSH |

For every incident, record UTC detection/containment/recovery times, affected
Goal IDs without payloads, snapshot/receipt identifiers, decisions, and measured
RPO/RTO. Preserve redacted audit evidence and rotate any credential whose
confidentiality cannot be proven.
