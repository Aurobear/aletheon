# Deployment secret lifecycle

Aletheon uses files below `/etc/aletheon/credentials` rather than command-line
arguments, checked-in configuration values, or a plaintext systemd
`EnvironmentFile` from persistent storage. The directory is `root:aletheon
0750`; credential files are `0600`. `LoadCredential=` gives the service private
read-only copies; integration `NAME=value` bundles are parsed only from those
copies. The service has
`LimitCORE=0`, so its decrypted process state is not written to a core dump.

## Inventory

| Credential | Owner | Canonical file | Rotation and revocation |
|---|---|---|---|
| Model provider keys | `aletheon:aletheon` | `provider.env` | Create overlapping key, replace, restart/test, revoke old key at provider |
| Telegram bot token | `aletheon:aletheon` | `telegram.env` | Revoke/regenerate with BotFather, replace, restart, test long poll |
| Google OAuth client secret | `aletheon:aletheon` | `provider.env` | Create client secret where overlap is supported, replace/test, delete old secret |
| Google encrypted-vault master key | `aletheon:aletheon` | `google-vault.key` | Re-encrypt every vault record under a new key before atomic replacement; loss makes OAuth records unrecoverable |
| GBrain bearer/database credentials | `aletheon:aletheon` | `gbrain.env` | Add overlapping bearer credential, replace/restart/test spool drain, revoke old credential |
| Restic repository password | `aletheon:aletheon` | `restic-password` | Add/test new repository key before removing the old key; retain separately encrypted recovery material |
| Restic repository reference | `aletheon:aletheon` | `restic-repository` | Replace only after the destination is initialized and a test snapshot succeeds |
| Tailscale node/host identity | root/Tailscale daemon | Tailscale state directory, not copied into Aletheon | Expire/remove node in tailnet admin, rotate auth key, re-enrol host |

Environment bundles contain only `NAME=value` records needed by their named
integration. Never combine unrelated credentials. Google refresh/access tokens
remain encrypted in the local vault and must not appear in these bundles.

## Initialize and validate

Run after the service account exists:

```sh
sudo /usr/libexec/aletheon/aletheon-secret-init.sh init
sudo /usr/libexec/aletheon/aletheon-secret-audit.sh --validate
```

The initializer writes the 32-byte vault key directly from `/dev/urandom` into
a root-created temporary file and atomically renames it. It neither prints the
key nor places it in an argument. Empty optional integration files are created
as placeholders. The systemd pre-start check rejects symlinks, non-regular
files, incorrect ownership/mode, world-writable ancestors, a malformed vault
key, or a credential tracked by the current Git checkout.

## Rotate

1. Create a second credential at the provider when overlap is supported.
2. Feed it over standard input, never as an argument:
   `sudo aletheon-secret-init.sh rotate provider.env < protected-file`.
3. Run `systemctl restart aletheon` and wait for `aletheon-healthcheck.sh`.
4. Exercise the affected integration and inspect only outcome/error codes.
5. Revoke the old credential and securely remove the protected input.

For the vault key, stop intake, back up the vault, decrypt and re-encrypt every
entry in a restricted staging directory, atomically install the new vault and
key, then validate before deleting the old encrypted material. Never rotate the
key alone.

## Canary audit and incident response

Create a non-production canary in a `0600` file, exercise each integration, then
run:

```sh
sudo /usr/libexec/aletheon/aletheon-secret-audit.sh \
  --canary-file /root/test.secret-canary
```

The audit checks Git-tracked content, service process argv/environment,
journald, audit/artifact/session data, caches, modes, and symlinks without
printing the canary. Inspect encrypted backups with their restore verifier as a
separate step. Support bundles must pass the same redactor used by audit/model
frames and remove Authorization/Cookie headers, token-like values, email bodies,
provider request/response payloads, credential paths, and environment dumps.
Do not collect `/proc/*/environ` or raw model transcripts in a support bundle.

On suspected disclosure: stop the affected integration, preserve redacted audit
evidence, revoke the provider credential, rotate it using the procedure above,
invalidate active sessions/tokens, run the canary audit, and restore service
only after readiness and integration validation succeed. Backups containing a
compromised vault key remain sensitive even after rotation.
