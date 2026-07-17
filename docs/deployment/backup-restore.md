# Encrypted backup and restore

## Recovery model

The daily timer snapshots all managed state under `/var/lib/aletheon`, policy
and non-secret configuration/policy under `/etc/aletheon`, and the encrypted
Google credential vault under state. `/etc/aletheon/credentials` is excluded and
its recovery material is protected separately. Live SQLite databases are copied with SQLite's online backup API; WAL and
SHM files are never copied independently. This covers Goals, attempts,
approvals, channel/outbox state, Google cursors/projections, Mnemosyne and GBrain
spool state, artifacts, worktrees, audit data, and local configuration.

That machine timer does not cover socket-activated user daemons. Their
systemd-managed durable root is `$HOME/.local/state/aletheon` (from the user
unit's `StateDirectory=aletheon`). A complete split-runtime migration backup
therefore consists of the machine snapshot plus a separately protected,
user-owned snapshot of that path for every enrolled account. Do not substitute
`$HOME/.local/share/aletheon`; it is not the unit's state directory. Per-user
cache under `$HOME/.cache/aletheon` and sockets under `/run/user/$UID` are
reconstructed rather than restored.

The manifest records UTC creation time, host ID, Aletheon/schema versions,
components, file sizes and SHA-256 hashes, and the Restic data snapshot ID. A
second small Restic snapshot stores the completed manifest as the receipt.
Backup logs contain snapshot identifiers and counts, never file contents or
credentials.

The Restic repository password and vault-key recovery copy must be stored in
separate encrypted systems. Loss of the Restic password makes every snapshot
unrecoverable. Loss of the Google vault key makes restored OAuth credentials
unusable. Compromise of both repository and recovery key exposes all retained
state; rotate provider credentials and rebuild the repository.

## Schedule and retention

Enable `aletheon-backup.timer` for daily local snapshots. Configure the Restic
repository to replicate weekly to a separately administered remote target. Run
weekly:

```sh
restic --repository-file /etc/aletheon/credentials/restic-repository \
  forget --keep-daily 14 --keep-weekly 8 --keep-monthly 12 --prune
restic --repository-file /etc/aletheon/credentials/restic-repository check
```

Set `RESTIC_PASSWORD_FILE` in the protected operator shell rather than adding a
password argument. A failed unit must notify the operator through the host's
systemd failure-notification mechanism; it must not delete the last successful
snapshot. Monitor readiness `backup` age against 36 hours.

## Restore

Restore only into a new empty staging root:

```sh
sudo systemctl stop aletheon-core.service
# As each enrolled user (or through an audited root-run user-manager helper):
systemctl --user stop aletheon.socket aletheon.service
sudo env ALETHEON_RESTORE_TARGET=/var/lib/aletheon.restore \
  ALETHEON_RESTORE_CONFIG_TARGET=/etc/aletheon.restore \
  ALETHEON_RESTORE_SNAPSHOT=latest /usr/libexec/aletheon/restore-aletheon.sh
```

The script refuses a nonempty target, validates the manifest and every hash,
optionally enforces `ALETHEON_SCHEMA_VERSION`, runs SQLite integrity checks
before and after installation, and removes group/world write permission. Review
ownership, restore the separately protected configuration/key material, move
the old root to a timestamped rollback directory, atomically place the staged
root, then start Aletheon and wait for readiness. Never run an older binary on a
newer schema without restoring its matching pre-upgrade snapshot.

For a split-runtime restore, keep the core and every affected user socket and
service stopped. Restore each matching user snapshot into an empty
`$HOME/.local/state/aletheon`, preserve its original UID/GID and restrictive
modes, run SQLite integrity checks there, then start the core followed by user
socket activation. Never combine restored machine state with migrated user
state, or vice versa.

## Release drills

For every release, back up while a Goal is active and while a WAL transaction is
present, restore to an empty disposable host, and verify Goal, approval, cursor,
memory, artifact, and audit continuity. Also test network loss, interrupted
Restic upload, wrong password, tampered manifest/file, corrupt SQLite, and
rollback to the untouched pre-restore root. Record snapshot IDs, elapsed time,
size, RPO, and RTO in the release evidence bundle. Use
`ALETHEON_BACKUP_MODE=staging` only for the local consistency smoke test; it is
not an encrypted production backup.

The installed-host release drill names two explicit disposable users and
retains machine and per-user integrity output before and after install,
restart, upgrade, and matching rollback.
