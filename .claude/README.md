# Claude Project Configuration

This directory stores project-level Claude configuration templates and operating notes for Aletheon.

## Contents

| File | Purpose |
|------|---------|
| `instructions.md` | Project-wide rules: branch/PR workflow, multi-agent coordination, crate conventions, safety invariants, dependency injection, test discipline, commit conventions, Phase 3-6 constraints |
| `settings.local.json` | Local permissions (git, gh, rg). Machine-specific — not committed |
| `settings.local.json.example` | Template for new contributors to copy |
| `worktrees/` | Ephemeral git worktrees created during multi-agent workflows (auto-cleaned) |

## Local setup

1. Copy the example file:

   ```bash
   cp .claude/settings.local.json.example .claude/settings.local.json
   ```

2. Edit `.claude/settings.local.json` for your local machine.

Do not commit real tokens, personal paths, or machine-specific secrets.

## Project workflow constraints

- Use `dev` as the integration branch unless a maintainer says otherwise.
- After a PR/MR is merged, delete the merged feature branch locally and remotely when safe.
- Do not delete branches that are still open, unmerged, protected, or shared by another active PR/MR.
- Prefer repository hosting settings such as "automatically delete head branches" when available.
- If branch deletion requires elevated credentials, ask the user or maintainer instead of forcing it.
