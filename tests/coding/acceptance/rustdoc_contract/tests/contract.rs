use rustdoc_contract::{parse_worker_count, ParseError};

#[test]
fn public_error_contract_covers_all_documented_cases() {
    assert_eq!(parse_worker_count(""), Err(ParseError::Empty));
    assert_eq!(parse_worker_count("0"), Err(ParseError::Zero));
    assert_eq!(parse_worker_count("abc"), Err(ParseError::Invalid));
    assert_eq!(parse_worker_count("65535"), Ok(65_535));
}

#[test]
fn documentation_links_to_the_public_error_type() {
    let source = include_str!("../src/lib.rs");
    assert!(source.contains("[`ParseError`]"));
    assert!(!source.contains("ParseFailure"));
}
