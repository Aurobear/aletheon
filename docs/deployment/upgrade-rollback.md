# Upgrade and rollback

## Upgrade

Stage a release containing the executable, detached release/checksum evidence,
configuration compatibility notes, asset `MANIFEST.sha256`, schema version, and
release notes. On a supported staging host, run the complete migration and
restore drill before production.

```sh
sudo upgrade-aletheon.sh \
  --binary ./release/aletheon \
  --sha256-file ./release/aletheon.sha256 \
  --assets ./release/assets
```

The script verifies the candidate and assets, records its version, creates an
encrypted backup while the current daemon is available, saves the installed
binary, stops intake, atomically installs the candidate, runs production config
preflight, starts forward migrations, waits for readiness, and writes a version
receipt under `/var/lib/aletheon/state/upgrades`. It fails before stopping the
service if backup or verification fails.

## Rollback

Schema compatibility decides the procedure:

* If release notes prove the schema remained backward-compatible, stop the
  daemon, verify the saved binary checksum, install it atomically, and start it.
* If a migration ran or compatibility is unknown, stop the daemon and restore
  the matching pre-upgrade snapshot into empty staging roots using
  `restore-aletheon.sh`. Preserve the failed upgraded root as evidence, place
  the restored data/config roots, install the matching saved binary, preflight,
  start, and verify readiness plus a representative Goal and integration.

Never point an old binary at a migrated database. Never remove the only known
good snapshot or overwrite the pre-restore directory. Record the binary hash,
snapshot ID, schema versions, start/readiness times, and operator in the release
evidence bundle.
