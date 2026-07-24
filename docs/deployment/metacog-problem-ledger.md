# Metacog append-only state operations

## Scope and ownership

Metacog owns append-only evidence, problem, improvement, experiment, and
lineage records. The daemon owns their configured state directory and file
permissions. Domain adapters may submit generic Fabric contracts but must not
write Metacog state files directly.

```text
domain adapter -> validated Fabric value -> Metacog service -> append event
                                                        |
                                                        v
                                               replay projection
```

Each durable record carries a schema version or is enclosed by a versioned
event. Writers append complete JSONL records. Readers reconstruct current state
in file order; they do not treat a mutable in-memory projection as canonical.

## Restart and replay

1. Stop the daemon that owns the state root.
2. Back up the complete state root before repair.
3. Validate that each non-empty line is valid JSON and that evidence integrity
   digests match the canonical payload.
4. Restart the daemon and allow stores to replay records in append order.
5. Verify record counts, latest lifecycle states, proposal authority, active
   experiments, and causal lineage links.

Duplicate identifiers are idempotent only when the canonical content matches.
A conflicting duplicate is corruption and must not silently replace the first
record.

## Corruption and quarantine

Malformed JSON, an invalid schema version, a failed integrity digest, an illegal
lifecycle transition, or a broken causal reference must fail closed. Do not
truncate the live log or skip a bad record and continue mutation processing.

Copy the affected file and its metadata into a timestamped, access-restricted
quarantine directory outside the active state path. Restore a reviewed backup
or repair by generating an explicit corrective event with an audited migration
tool. Preserve the original bytes for diagnosis.

## Backup and restore

Use the repository deployment backup procedure while the owning daemon is
quiesced:

```bash
sudo bash scripts/aletheon.sh backup
sudo bash scripts/aletheon.sh restore
```

Restore all related logs as one consistency unit. Restoring only a problem log
without its evidence, proposals, experiments, and lineage can leave valid JSON
with invalid causal state.

## Retention

Problem and lineage history is audit data and is not compacted by rewriting the
canonical log. If retention becomes necessary, create a signed/versioned
checkpoint, preserve the source backup, and append a receipt describing the
covered range. Active, disputed, regressed, unresolved, or experiment-linked
records must remain recoverable.

## Redaction

Domain adapters must redact secrets before evidence ingestion and set redaction
metadata. Never store provider credentials, tokens, private keys, raw secret
files, or unrestricted user content in evidence payloads. A later redaction
does not erase already appended bytes; quarantine and rotate affected storage,
then append a redacted replacement record with provenance.

## Acceptance

Operational acceptance requires:

- restart replay preserves evidence, problems, proposals, experiments, and
  lineage;
- corrupt or conflicting records fail closed;
- unknown evidence remains unknown rather than becoming failure;
- no proposal self-approves;
- hard gates block promotion regardless of score;
- rollback and causal lineage remain available.
