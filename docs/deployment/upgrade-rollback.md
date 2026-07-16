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

## Release compatibility matrix

The authoritative per-component compatibility declaration is
`config/release/migration-matrix.toml`. Run
`scripts/verify-migration-matrix.sh` before staging a candidate. The verifier
requires every durable component to declare its source and target version,
backup boundary, forward action, integrity evidence and rollback method.

A transition marked `data_change = true` is never eligible for binary-only
rollback. Stop the upgraded daemon, retain its data root as evidence, restore
the pre-upgrade snapshot into empty roots, install the binary saved with that
snapshot, then run preflight, readiness and representative V01 checks. Mixed
old/new daemon operation against the same durable roots is forbidden.

The production drill runs only inside a disposable systemd VM/container. It
records installation modes, AF_UNIX exposure, journal output, upgrade receipts,
SQLite integrity results, and the matching data+binary rollback receipt. A
missing disposable host, release binary, V01 report, production credential or
operator identity blocks release; it is not an ignored case.
