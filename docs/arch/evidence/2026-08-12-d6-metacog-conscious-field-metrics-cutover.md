# D6 Metacog conscious-field metrics cutover

The three deterministic conscious-field evidence models and their computations
now live in `metacog::evaluation::conscious_field_metrics`. Executive consumes
that owner API; Fabric no longer defines or exports the metrics aggregate.

Evidence:

- production and acceptance callers are limited to Executive;
- the target owner recorded in the Fabric boundary census was Metacog;
- `rg 'conscious_field_metrics|FieldMetric' crates/fabric/src` returns no match;
- the Fabric public inventory and boundary census remove all three types.
