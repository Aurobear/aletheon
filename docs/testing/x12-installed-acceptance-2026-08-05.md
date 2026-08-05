# X12 installed acceptance evidence — 2026-08-05

Status: `code_complete` (the installed provenance lanes pass; the 20-task
engineering success threshold is still open).

## Installed runtime lanes

Command:

```text
sudo bash scripts/aletheon.sh deploy
```

Evidence from the system-installed runtime:

- release build/install/restart completed;
- `/usr/bin/aletheon`, machine core, user daemon, and Memory Agent provenance
  matched at SHA-256 `c6e480aef293f7d8712ccbf8bb05aa826a76c742615f4acb36a09f45a41e9c01`;
- system and user `NRestarts=0` remained stable;
- official Memory Agent protocol smoke passed;
- an official user-socket real request passed;
- deployment verification passed.

These facts satisfy the installed provenance and real-request portions of
`U-INST-001` and `U-INST-002`. They do not imply the corpus threshold.

## Corpus run diagnosis

The first installed corpus run used `/usr/bin/aletheon` and the official user
socket. Early receipts exposed a runner contract defect: the installed client
returns a versioned terminal envelope (`type=terminal`, `status`, nested
`metrics`) while the runner only recognized the older flattened `stop` shape.
The runner now normalizes both shapes in `tests/coding/harness/run.py` and has a
regression test in `tests/coding/runner_test.py`.

The run also recorded genuine runtime timeouts/provider failures while the
installed daemon was recovering its populated event spine. Those receipts are
retained under the local `target/coding-x12-20260805/` artifact directory and
are not presented as U-INST-003 success. A fresh corpus run after the runner
fix is required to decide `U-INST-003`.
