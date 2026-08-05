use fixture_rust_bugfix::*;

#[test]
fn hidden_boundary_contract() { assert_eq!(take_limit(&[1,2,3], 2), vec![1,2]); }
