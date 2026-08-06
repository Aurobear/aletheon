use fixture_rust_bugfix::take_limit; #[test] fn exact_limit_keeps_exact_count(){assert_eq!(take_limit(&[1,2,3,4],3),vec![1,2,3]);}
