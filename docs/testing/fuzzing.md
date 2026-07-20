# Fuzz testing

The executive fuzz package contains seven parsing and protocol targets under
`crates/executive/fuzz/fuzz_targets`.

Run a short local campaign through the repository Cargo wrapper:

```bash
for target in \
  envelope_v2_parse envelope_v2_roundtrip \
  jsonrpc_message_parse jsonrpc_method_dispatch \
  toml_config_parse tool_input_json message_roundtrip
do
  bash scripts/cargo-agent.sh fuzz run "$target" \
    --fuzz-dir crates/executive/fuzz -- -max_total_time=30
done
```

For an extended campaign, increase `-max_total_time` or omit it and stop the
fuzzer manually. Preserve any discovered reproducer in the target's corpus and
run that target again after fixing the crash. Fuzz targets treat malformed
input as an expected error; panics, aborts, hangs, and round-trip mismatches are
failures.
