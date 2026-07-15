# Production filesystem and configuration contract

Aletheon has three path modes:

- `development`: relative/project paths are allowed for tests and local builds;
- `user`: backwards-compatible `~/.aletheon` and XDG paths are expanded by the
  interactive process;
- `production`: every path is absolute, normalized, and confined to the roots
  below. An unresolved `~`, traversal, world-writable directory, or symlinked
  credential root is rejected.

```text
/etc/aletheon/                  root:aletheon 0750
├── config.toml                 root:aletheon 0640
├── policy/                     root:aletheon 0750
└── credentials/                root:aletheon 0750
    └── individual keys/files   aletheon:aletheon 0600

/var/lib/aletheon/              aletheon:aletheon 0750
├── state/
│   └── snapshots/
├── goals/
├── sessions/
├── mnemosyne/
├── artifacts/
├── worktrees/
└── audit/

/var/cache/aletheon/            aletheon:aletheon 0750
/run/aletheon/                  aletheon:aletheon 0750
└── aletheon.sock               aletheon:aletheon 0660
```

`/run/aletheon` is canonical. Linux normally provides `/var/run -> /run`, so
older clients keep working through the operating-system symlink; Aletheon does
not maintain a second runtime directory.

Use [`config/production.toml.example`](../../config/production.toml.example) as
the production overlay. It types every state path, quota, integration switch,
secret-file reference, backup mode, and health threshold. Inline secrets are
not configuration fields.

Only documented non-secret settings may be templated by an installer or host
environment: quota/health numbers, integration enable flags, log level, and the
four canonical roots. Secret values must be read from the referenced regular
files. Paths under production mode must still pass runtime validation after any
templating.

Pre-install validation permits missing directories so an idempotent installer
can create them. Service-start validation requires them to exist with safe
ownership/modes. Never resolve production `~` using the service account home,
and never relocate state outside `/var/lib/aletheon` merely to make startup
succeed.
