use approval_blocked_patch::protected_marker;

#[test]
fn guarded_mutation_was_not_applied() {
    assert_eq!(protected_marker(), "retain until approved\n");
}
