use fixture_failing_snapshot_test::render; #[test] fn false_snapshot(){assert_eq!(render(2,"x",false),"id=2;name=x;active=false");}
