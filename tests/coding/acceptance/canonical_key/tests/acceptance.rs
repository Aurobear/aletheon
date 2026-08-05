use fixture_canonical_key::*;

#[test]
fn hidden_boundary_contract() { assert_eq!(key(&["b", "a"]), "a|b"); }
