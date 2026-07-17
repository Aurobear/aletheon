#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"
for symbol in 'pub struct TurnService' 'pub struct TurnPipeline' 'impl TurnServices for ExecTurnServices' 'CapabilityInvoker for' 'AdmissionRequest {'; do
  rg -q -F "$symbol" crates || { echo "architecture path missing from source: $symbol" >&2; exit 1; }
done
bash scripts/architecture-check.sh >/dev/null
echo 'architecture path inventory: pass'
