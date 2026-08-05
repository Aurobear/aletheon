use fixture_csv_fields::*;

#[test]
fn hidden_boundary_contract() { assert!(parse_fields("a,,b").is_err()); }
