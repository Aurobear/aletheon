use fixture_retry_backoff::*;

#[test]
fn hidden_boundary_contract() { assert_eq!(backoff(20), 1000); }
