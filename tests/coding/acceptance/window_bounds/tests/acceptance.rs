use fixture_window_bounds::*;

#[test]
fn hidden_boundary_contract() { assert_eq!(window(&[1,2,3,4], 1, 3), vec![2,3,4]); }
