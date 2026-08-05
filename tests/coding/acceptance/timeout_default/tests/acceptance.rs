use fixture_timeout_default::*;

#[test]
fn hidden_boundary_contract() { assert_eq!(timeout(0), 30000); }
