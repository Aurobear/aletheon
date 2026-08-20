# AK2-25 fabric-client retirement evidence

Date: 2026-08-13

## Requirement receipt

- AK2-25 requires the Fabric compatibility path to reach hard zero before final retirement: .
- Gateway protocol/client/server remain independent packages; this change does not merge those responsibilities: .

## Cutover

- The legacy JSON-line framing adapter moved from the obsolete `fabric-client` package into `gateway-client::legacy`.
- The Memory Agent client now consumes `gateway_client::LegacyProtocolClient`.
- The workspace member, package dependency, architecture dependency rows, and physical `crates/fabric-client` directory are deleted.
- The typed `gateway-protocol`, `gateway-client`, and `gateway-server` packages remain separate.

## Focused validation

```text
bash scripts/cargo-agent.sh check -p gateway-client --all-targets PASS
bash scripts/cargo-agent.sh check -p interact --all-targets PASS
bash scripts/cargo-agent.sh check -p aletheon --all-targets PASS
```
