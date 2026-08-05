use fixture_module_path_explanation::normalize_key;
#[test]
fn trims_and_lowercases() { assert_eq!(normalize_key(" Hello World "), "hello-world"); }
