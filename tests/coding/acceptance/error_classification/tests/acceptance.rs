use fixture_error_classification::*;

#[test]
fn hidden_boundary_contract() { assert_eq!(classify(""), "invalid"); }
