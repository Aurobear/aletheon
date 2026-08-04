# Native systemd deployment

Aletheon has two native service boundaries:

```
root-managed aletheon-core.service
  /run/aletheon/core.sock (0660, aletheon group)
                ^
                |
user-manager aletheon.socket -> aletheon.service
  %t/aletheon/aletheon.sock (0600, owning user)
                |
      arbitrary per-turn workspaces

user-manager aletheon-memory-agent.service
  /usr/bin/aletheon memory-agent serve --official-user-socket
                |
       private socket only; no workspace/DB/credential access
```

The machine service runs only `aletheon core`. Each authorized local user gets a
private, socket-activated `aletheon daemon`; Goal workers, integrations, and Pi
remain supervised inside that user runtime. The bounded memory maintenance
scheduler is separate so runtime/provider failure cannot stall the daemon; all
policy, database, binding, and receipt authority remains behind its socket.

For the supported build/install/deploy command surface, start with the
[operations guide](README.md):

```bash
bash scripts/aletheon.sh deploy
```

The dispatcher builds through the bounded Cargo wrapper, calls the native
installer at its explicit sudo boundary, installs the per-user closure assets,
and verifies readiness. `scripts/libexec/aletheon/install-systemd.sh` remains the low-level
installer for image construction and advanced automation.

The installer is idempotent. It creates the system account and managed 0750
core directories, preserves an existing `/etc/aletheon/config.toml`, installs
only checked-in assets, runs typed core and user-unit verification, enables the
core, and globally enables the user socket for user managers. Use `--no-enable`
for image construction or offline verification.

## Authorize a user

Core inference is group-authorized. Installation deliberately does not grant
all local accounts access. Enroll each approved user explicitly, then refresh
that user's login session so supplementary groups are reapplied:

```bash
sudo usermod -aG aletheon USER
# log out and back in, then as USER:
systemctl --user enable --now aletheon.socket
systemctl --user enable --now aletheon-memory-agent.service
```

Global socket enablement makes the unit available when a user manager starts;
it does not keep every user's runtime resident. If an approved account must be
available without an interactive login, enable lingering as an explicit
operator decision:

```bash
sudo loginctl enable-linger USER
```

The private socket is `%t/aletheon/aletheon.sock`, normally
`/run/user/$UID/aletheon/aletheon.sock`. It is owned by that user with mode
0600. The daemon adopts the socket-activation descriptor and starts only when a
client connects. The socket unit exclusively owns the runtime directory; the
service must not declare the same `RuntimeDirectory`, otherwise a service
restart can unlink the still-active listener.

## Security and credentials

The core unit uses `ProtectSystem=strict`, an empty capability set, and
read/write access only to `/run/aletheon`, `/var/lib/aletheon`, and
`/var/cache/aletheon`. Root manages provider credentials in
`/etc/aletheon/credentials/provider.env`; never put a token in unit text,
`ExecStart`, or `Environment=`. User integration credentials and user runtime
state remain in that user's authority domain.

For the checked-in user unit, systemd expands `StateDirectory=aletheon` to
`$XDG_STATE_HOME/aletheon`, normally `$HOME/.local/state/aletheon`. It expands
`CacheDirectory=aletheon` to `$XDG_CACHE_HOME/aletheon`, normally
`$HOME/.cache/aletheon`. Do not treat `$HOME/.local/share/aletheon` as the
managed state directory. The private socket remains below `$XDG_RUNTIME_DIR`
and is not durable state.

The Memory Agent runs the same installed executable as the daemon. Maintenance
capability is granted only when `SO_PEERCRED`, `/proc/PID/exe`, and the exact
`memory-agent serve --official-user-socket` argv agree. Its unit has strict
system protection, read-only home visibility, no state/cache directory, no
credential environment, no writable workspace, and only `AF_UNIX` access. It
never opens Mnemosyne or GBrain storage directly.

Pi/bubblewrap requires user and mount namespaces, so `RestrictNamespaces` is
intentionally not set on the relevant runtime. `NoNewPrivileges`, filesystem
protection, and the application sandbox remain active. Re-run Pi
namespace/worktree tests after changing hardening directives.

The system backup and cleanup timers cover only machine-scoped core state below
`/var/lib/aletheon` and `/var/cache/aletheon`. They do **not** back up or delete
systemd-managed per-user state/cache directories. Per-user data needs a
separate user-owned retention and backup policy. A matching per-user backup
must capture `$HOME/.local/state/aletheon` for every enrolled principal; cache
and runtime socket directories are reconstructible and are not authoritative
rollback inputs.

## Validation and recovery

```bash
scripts/aletheon.sh verify systemd --core-unit config/aletheon-core.service \
  --binary target/release/aletheon
scripts/aletheon.sh verify systemd --user-units \
  config/aletheon.user.service config/aletheon.user.socket \
  config/aletheon-memory-agent.user.service \
  --binary target/release/aletheon
scripts/aletheon.sh verify systemd --preflight --binary target/release/aletheon \
  --config config/production.toml.example

sudo systemctl restart aletheon-core.service
sudo systemctl status aletheon-core.service --no-pager
sudo journalctl -u aletheon-core.service -n 200 --no-pager

# Run these as the authorized user. Stop the activated service before cycling
# its listener, then let the next client connection start the service again.
systemctl --user stop aletheon-memory-agent.service aletheon.service aletheon.socket
systemctl --user start aletheon.socket
systemctl --user start aletheon-memory-agent.service
systemctl --user status aletheon.socket aletheon.service \
  aletheon-memory-agent.service --no-pager
journalctl --user -u aletheon.service -n 200 --no-pager
journalctl --user -u aletheon-memory-agent.service -n 200 --no-pager
scripts/aletheon.sh verify systemd --readiness \
  --socket "$XDG_RUNTIME_DIR/aletheon/aletheon.sock"
```

Static verification is not production evidence. Test two distinct users,
cross-user socket denial, crash restart, and SIGTERM during active work on a
disposable systemd host before rollout.
