use clippy_cleanup::is_supported_port;

#[test]
fn cleanup_does_not_widen_or_narrow_the_range() {
    assert!(!is_supported_port(0));
    assert!(!is_supported_port(u16::MAX));
    assert_eq!(
        (1020..=1028)
            .filter(|port| is_supported_port(*port))
            .collect::<Vec<_>>(),
        [1024, 1025, 1026, 1027, 1028]
    );
}

#[test]
fn cleanup_does_not_suppress_the_lint() {
    let source = include_str!("../src/lib.rs");
    assert!(!source.contains("allow(clippy"));
}
