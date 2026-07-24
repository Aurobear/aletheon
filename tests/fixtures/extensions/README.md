# Extension Platform Fixtures

These fixtures are the checked-in baseline and malicious-package corpus used by
the extension inspector security gate.

## Packages

- **legal-minimal**: A minimal valid extension package for baseline testing.
  Contains a single `skill` asset (`skill.demo`) with a valid SKILL.md.

- **malicious-\*** directories: readable manifest/checksum fixtures used by
  parser and path-validation tests.
- **archives/\*.tar.gz**: deterministic archive fixtures for every mandatory
  R1 negative case: symlink, hardlink, FIFO, device header, duplicate entry,
  duplicate checksum, non-hex checksum, undeclared file, missing asset, and
  asset-kind/path mismatch.

## Usage

`required_malicious_archive_fixtures_are_rejected_without_staging_output`
opens every archive through the production `extract_to_staging` entry point.
Every fixture must fail before staging is created, and the test also asserts
that no outside path is produced.
