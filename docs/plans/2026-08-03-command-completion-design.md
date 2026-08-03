# Command Completion and Session Resume Design

## Problem

Completion currently has three disconnected behaviors:

- TUI command candidates are visible, but `Tab` cycles rather than accepting
  the selected command (`crates/interact/src/tui/app/key_handler.rs:381-400`).
- `/sessions` opens the existing canonical session picker, while `/resume`
  without an ID bypasses it (`crates/interact/src/tui/app/submit.rs:175-215`).
- The checked-in Bash/Zsh completion definitions register only the operations
  script name `aletheon.sh`, not the installed `/usr/bin/aletheon` CLI
  (`scripts/completions/aletheon.bash:59`,
  `scripts/completions/aletheon.zsh:58`).

`setup.sh` installs those definitions (`setup.sh:484-506`), but the supported
system deployment path does not install any completion assets
(`scripts/libexec/aletheon/install-systemd.sh:31-76`). A completion definition
must be sourced by a shell or loaded from its standard completion directory;
executing it as a child Bash process cannot modify the parent shell.

## Goals

1. Make `Tab` accept the selected TUI slash-command candidate.
2. Make bare `/resume` display the existing authoritative session picker.
3. Add Bash and Zsh completion for the installed `aletheon` CLI while retaining
   completion for the repository operations script `aletheon.sh`.
4. Install completion definitions as part of both system and rootless deploys;
   deployment, not a manual source command, owns persistent installation.

## TUI behavior

```text
/mem<Tab>       -> /memory
/res<Tab>       -> /resume
Up / Down       -> choose a visible candidate
Shift+Tab       -> select the previous candidate
Enter           -> accept the selected candidate (compatibility)

/resume         -> open canonical session picker
/sessions       -> open the same picker
/resume <id>    -> directly resume a known ID
```

Accepted commands receive a trailing space only when their registry usage
requires arguments. Commands with no arguments remain exact, preventing an
invisible trailing space from changing command parsing.

## Shell completion assets

Keep separate definitions because the two programs have different contracts:

```text
scripts/completions/aletheon.bash       installed CLI: aletheon
scripts/completions/aletheon.zsh        installed CLI: aletheon
scripts/completions/aletheon-ops.bash   repository operations: aletheon.sh
scripts/completions/aletheon-ops.zsh    repository operations: aletheon.sh
```

The CLI definitions cover Clap's public command tree and common value choices,
including `exec`, `config`, `doctor`, `extension`, `memory-agent`, and `memory`.
They also complete file/directory arguments where their type is known. Hidden
test instrumentation is excluded.

System deployment installs:

```text
/usr/share/bash-completion/completions/aletheon
/usr/share/bash-completion/completions/aletheon.sh
/usr/share/zsh/site-functions/_aletheon
/usr/share/zsh/site-functions/_aletheon.sh
```

Rootless deployment installs the same four logical assets below
`$XDG_DATA_HOME` (or `~/.local/share`). Files remain mode `0644`; completion
definitions are data sourced by the shell, not executables.

## Deployment integration

Create one reviewed installer helper used by `setup.sh` and
`scripts/lib/aletheon/install.sh`. System installation runs inside the existing
sudo boundary; user installation writes only below the user's data home.
Deployment verifies that both the CLI and operations completion names are
installed and contain their corresponding registration statement.

An already-running shell may require `source` or a new shell because shells
cache completion definitions. That is activation behavior, not a missing
deployment asset. Deployment prints a concise activation hint without asking
the user to chmod or execute the definition.

## Safety and boundaries

- Completion never executes Aletheon commands or queries providers.
- Session candidates come only from the existing `sessions` RPC and picker.
- `/resume <id>` remains supported for automation.
- Absolute paths that begin with `/` must continue through the existing
  slash-command disambiguation and must not be rewritten as commands.
- Completion files contain no credentials or machine-specific paths.

## Validation

1. TUI key tests prove `Tab` acceptance, arrow/Shift-Tab selection, disabled
   candidate rejection, and absolute-path preservation.
2. Resume tests prove bare `/resume` and `/sessions` request the session list,
   while `/resume <id>` sends the direct resume request.
3. Shell tests source Bash completion in a clean non-interactive shell and
   assert candidates for `aletheon`, nested memory commands, and `aletheon.sh`.
4. Static Zsh tests verify both `compdef` registrations and command trees.
5. `sudo bash scripts/aletheon.sh deploy` must install all four assets, pass
   installed-runtime provenance, and retain stable daemon restart counters.
