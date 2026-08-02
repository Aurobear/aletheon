use budget_exhaustion::weighted_total;

#[test]
fn insufficient_budget_does_not_apply_a_speculative_change() {
    assert_eq!(weighted_total(&[1, 2]), 6);
    assert!(include_str!("../SPEC.md").contains("negative-number behavior"));
}
