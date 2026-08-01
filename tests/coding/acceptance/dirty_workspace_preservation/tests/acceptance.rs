use dirty_workspace_preservation::normalize_label;

#[test]
fn repair_handles_whitespace_boundaries() {
    assert_eq!(normalize_label("\tvalue\n"), "value");
    assert_eq!(normalize_label("value"), "value");
}

#[test]
fn user_owned_dirty_content_is_still_exact() {
    assert_eq!(
        include_str!("../notes.txt"),
        "user-owned draft: keep this exact line\n"
    );
}
