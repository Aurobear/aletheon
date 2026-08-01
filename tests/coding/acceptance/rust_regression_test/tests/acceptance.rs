use rust_regression_test::{split_escaped, ParseError};

#[test]
fn escaped_delimiters_and_backslashes_are_preserved() {
    assert_eq!(
        split_escaped(r"alpha\,beta,path\\name").unwrap(),
        ["alpha,beta", r"path\name"]
    );
    assert_eq!(split_escaped("alpha\\"), Err(ParseError::TrailingEscape));
}

#[test]
fn repair_includes_a_visible_regression_test() {
    let source = include_str!("../src/lib.rs");
    assert!(source.matches("#[test]").count() >= 3);
}
