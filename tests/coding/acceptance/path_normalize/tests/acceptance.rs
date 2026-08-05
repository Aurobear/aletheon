use fixture_path_normalize::*;

#[test]
fn hidden_boundary_contract() { assert_eq!(normalize("/a///b"), "/a/b"); }
