# Native systemd deployment

Aletheon Executive is the only native application service. Goal workers,
Telegram, Google sync, Pi, and memory workers remain supervised inside
Executive; do not create one unit per worker.

Build a release binary, review `config/production.toml.example`, then install:

```bash
cargo build --release -p aletheon-bin
sudo ALETHEON_BINARY="$PWD/target/release/aletheon" \
  ALETHEON_CONFIG="$PWD/config/production.toml.example" \
  scripts/install-systemd.sh
```

The installer is idempotent: it creates the system account and managed
0750 directories, preserves an existing `/etc/aletheon/config.toml`, installs
only checked-in assets, validates the real `daemon --config`/`--socket` flags,
runs `systemd-analyze verify`, then enables the unit. Use `--no-enable` for
image construction or offline verification.

The unit uses `ProtectSystem=strict`, an empty capability set, read/write access
only to state/cache/runtime roots, and no plaintext `EnvironmentFile`. systemd
credentials are mounted read-only below `$CREDENTIALS_DIRECTORY`; application
secret lifecycle and rotation are described separately. Never put a token in
unit text, `ExecStart`, or `Environment=`.

Pi/bubblewrap requires user and mount namespaces, so `RestrictNamespaces` is
intentionally not set. This is the minimal exception; `NoNewPrivileges`,
`RestrictSUIDSGID`, filesystem protection, and the application sandbox remain
active. Re-run Pi namespace/worktree tests after changing hardening directives.

The preflight rejects missing/symlinked configuration, unsafe modes, non-
production mode, and noncanonical roots. `ExecStartPost` sends the real JSON-RPC
`health` request through the credential-checked Unix socket. Watchdog is not
configured because the daemon does not yet emit systemd heartbeat notifications.

Validation and recovery:

```bash
systemd-analyze verify config/aletheon.service
scripts/verify-systemd.sh --preflight --binary target/release/aletheon \
  --config config/production.toml.example
sudo systemctl restart aletheon
sudo systemctl status aletheon --no-pager
sudo journalctl -u aletheon -n 200 --no-pager
```

`SIGTERM` receives 30 seconds for bounded worker/connection drain, after which
`KillMode=mixed` terminates remaining children. Five failures within five
minutes trigger start limiting. Test crash restart and SIGTERM during an active
Goal on a disposable host before production rollout.
