use fixture_timeout_saturation_bugfix::deadline_ms;
#[test] fn overflow_saturates(){ assert_eq!(deadline_ms(u64::MAX-2,9),u64::MAX); }
