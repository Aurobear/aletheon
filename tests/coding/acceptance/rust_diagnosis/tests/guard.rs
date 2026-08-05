use fixture_rust_diagnosis::attempts; #[test] fn zero_retries_still_attempts_once(){assert_eq!(attempts(0),1);}
