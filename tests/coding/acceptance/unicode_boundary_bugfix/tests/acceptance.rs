use fixture_unicode_boundary_bugfix::truncate_chars;
#[test] fn unicode_scalars_are_preserved(){ assert_eq!(truncate_chars("你a好",2),"你a"); assert_eq!(truncate_chars("🙂x",1),"🙂"); }
