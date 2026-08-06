use fixture_parser_domain_sync::{domain::Priority, format::display, parser::parse};

#[test]
fn contract_is_typed_and_canonical() {
    for (raw, expected, rendered) in [
        (" LOW ", Priority::Low, "low"),
        ("Normal", Priority::Normal, "normal"),
        ("HIGH", Priority::High, "high"),
    ] {
        let priority = parse(raw).unwrap();
        assert_eq!(priority, expected);
        assert_eq!(display(&priority), rendered);
    }
    assert!(parse("urgent").is_err());
}
