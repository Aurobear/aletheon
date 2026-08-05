use fixture_config_precedence_explanation::config::*;
#[test] fn project_wins(){ assert_eq!(effective(Config{timeout:10},Some(20),Some(30)).timeout,30); }
