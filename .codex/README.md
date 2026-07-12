# Codex Project Configuration

This directory stores project-level Codex configuration templates and operating notes for Aletheon.

## Local setup

1. Copy the example file:

   ```bash
   cp .codex/config.toml.example .codex/config.toml
   ```

2. Edit `.codex/config.toml` for your local machine.

Do not commit real tokens, personal paths, or machine-specific secrets.

## Project workflow constraints

- Use `dev` as the integration branch unless a maintainer says otherwise.
- After a PR/MR is merged, delete the merged feature branch locally and remotely when safe.
- Do not delete branches that are still open, unmerged, protected, or shared by another active PR/MR.
- Prefer repository hosting settings such as "automatically delete head branches" when available.
- If branch deletion requires elevated credentials, ask the user or maintainer instead of forcing it.
