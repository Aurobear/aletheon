use fixture_rust_multifile::parser::parse_label;

#[test]
fn comma_separated_labels_are_normalized_and_serialized() {
    let parsed = parse_label("alpha, beta,alpha").unwrap();
    assert_eq!(parsed.0, "alpha,beta");
}

#[test]
fn empty_segment_is_rejected() {
    assert!(parse_label("alpha,,beta").is_err());
}
