use rustdoc_contract::{parse_worker_count, ParseError};

#[test]
fn zero_is_rejected_by_the_documented_contract() {
    assert_eq!(parse_worker_count("0"), Err(ParseError::Zero));
    assert_eq!(parse_worker_count("2"), Ok(2));
}
