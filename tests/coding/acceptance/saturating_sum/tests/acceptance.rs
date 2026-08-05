use fixture_saturating_sum::*;

#[test]
fn hidden_boundary_contract() { assert_eq!(sum(&[u32::MAX, 1]), u32::MAX); }
