# X12 installed acceptance evidence — 2026-08-05

Status: `failed` (the installed provenance lanes pass, but the recorded
20-task engineering run reached 18/20 and does not satisfy the release gate).

## Installed runtime lanes

Command:

```text
sudo bash scripts/aletheon.sh deploy
```

Evidence from the system-installed runtime:

- release build/install/restart completed;
- `/usr/bin/aletheon`, machine core, user daemon, and Memory Agent provenance
  matched at SHA-256 `caf149d3c04d5243e9c7c3d66ccdb4d0923018bf0fd6103683ccfca15cbd6db3`;
- system and user `NRestarts=0` remained stable;
- official Memory Agent protocol smoke passed;
- an official user-socket real request passed;
- deployment verification passed.

These facts satisfy the installed provenance and real-request portions of
`U-INST-001` and `U-INST-002`. They do not imply the corpus threshold.

## Corpus run diagnosis

The first installed corpus run used `/usr/bin/aletheon` and the official user
socket. Early receipts exposed a runner contract defect: the installed client
returns a canonical versioned terminal envelope (`type=terminal`, `status`,
nested `metrics`) while the runner read obsolete flattened fields. The runner
now validates and consumes the canonical v1 envelope in
`tests/coding/harness/run.py`, with regression coverage in
`tests/coding/runner_test.py`.

The first two runs are not presented as U-INST-003 success: one changed runtime
generation mid-suite, and the next exposed the auto-sandbox environment and
writable-mount defects tracked as XF-002. A later run was also stopped when a
concurrent checkout redeployed a different binary during the suite. A fresh,
single-generation corpus run is required to decide `U-INST-003`.
