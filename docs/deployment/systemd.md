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
```

The machine service runs only `aletheon core`. Each authorized local user gets a
private, socket-activated `aletheon daemon`; Goal workers, integrations, Pi, and
memory workers remain supervised inside that user runtime rather than becoming
separate system units.

Build a release binary, review `config/production.toml.example`, then install:

```bash
cargo build --release -p aletheon-bin
sudo ALETHEON_BINARY="$PWD/target/release/aletheon" \
  ALETHEON_CONFIG="$PWD/config/production.toml.example" \
  scripts/install-systemd.sh
```

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
scripts/verify-systemd.sh --core-unit config/aletheon-core.service \
  --binary target/release/aletheon
scripts/verify-systemd.sh --user-units \
  config/aletheon.user.service config/aletheon.user.socket \
  --binary target/release/aletheon
scripts/verify-systemd.sh --preflight --binary target/release/aletheon \
  --config config/production.toml.example

sudo systemctl restart aletheon-core.service
sudo systemctl status aletheon-core.service --no-pager
sudo journalctl -u aletheon-core.service -n 200 --no-pager

# Run these as the authorized user. Stop the activated service before cycling
# its listener, then let the next client connection start the service again.
systemctl --user stop aletheon.service aletheon.socket
systemctl --user start aletheon.socket
systemctl --user status aletheon.socket aletheon.service --no-pager
journalctl --user -u aletheon.service -n 200 --no-pager
scripts/verify-systemd.sh --readiness \
  --socket "$XDG_RUNTIME_DIR/aletheon/aletheon.sock"
```

Static verification is not production evidence. Test two distinct users,
cross-user socket denial, crash restart, and SIGTERM during active work on a
disposable systemd host before rollout.
