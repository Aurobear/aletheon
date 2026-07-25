#!/usr/bin/env bash
set -euo pipefail
env_file=${1:-deploy/gbrain/.env}
base=deploy/gbrain/compose.yaml
overlay=deploy/compose.production.yaml

[[ -f "$env_file" && ! -L "$env_file" ]] || { echo "missing/symlinked env file" >&2; exit 1; }
set -a
# shellcheck disable=SC1090
source "$env_file"
set +a
[[ ${GBRAIN_IMAGE_DIGEST:-} =~ ^sha256:[0-9a-f]{64}$ ]] || {
  echo "GBRAIN_IMAGE_DIGEST must be an immutable sha256 digest" >&2; exit 1;
}
[[ ${GBRAIN_BRAIN_DIR:-} == /var/lib/aletheon/gbrain/brain ]] || exit 1
[[ ${GBRAIN_DATABASE_DIR:-} == /var/lib/aletheon/gbrain/database ]] || exit 1
[[ ${GBRAIN_READ_TOKEN_FILE:-} == /etc/aletheon/credentials/* ]] || exit 1
[[ ${GBRAIN_WRITE_TOKEN_FILE:-} == /etc/aletheon/credentials/* ]] || exit 1

python3 - "$base" "$overlay" <<'PY'
import pathlib, sys, yaml
base, overlay = (yaml.safe_load(pathlib.Path(p).read_text()) for p in sys.argv[1:])
service = base['services']['gbrain']
assert '@${GBRAIN_IMAGE_DIGEST' in service['image']
assert service['ports'] == ['127.0.0.1:${GBRAIN_PORT:-9020}:9020']
assert service['read_only'] is True and service['cap_drop'] == ['ALL']
assert service['security_opt'] == ['no-new-privileges:true']
assert service['pids_limit']
assert base['networks']['memory']['internal'] is True
assert service['healthcheck'] and service['restart'] == 'unless-stopped'
assert service['logging']['options'] == {'max-size': '10m', 'max-file': '3'}
for volume in ('gbrain_brain', 'gbrain_database'):
    assert base['volumes'][volume]['driver_opts']['o'] == 'bind'
assert overlay['services']['gbrain']['pull_policy'] == 'never'
PY

if command -v docker >/dev/null 2>&1; then
  docker compose --env-file "$env_file" -f "$base" -f "$overlay" config >/dev/null
  rendered=$(docker compose --env-file "$env_file" -f "$base" -f "$overlay" config)
  grep -q '127.0.0.1:' <<<"$rendered"
  ! grep -Eq 'image: .*(:latest|REPLACE_)' <<<"$rendered"
else
  echo "verify-compose: docker unavailable; static contract passed, runtime drill pending" >&2
fi
