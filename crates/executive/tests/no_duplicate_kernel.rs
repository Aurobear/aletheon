use std::path::PathBuf;

#[test]
fn executive_has_no_duplicate_kernel_authority() {
    let executive = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(
        !executive.join("src/impl/kernel").exists(),
        "Executive-local Process/supervisor/token kernel must stay deleted"
    );
    let module = std::fs::read_to_string(executive.join("src/impl/mod.rs")).unwrap();
    assert!(!module.lines().any(|line| line.trim() == "pub mod kernel;"));

    // Replacement ownership is intentionally explicit rather than silently
    // dropping the old module's unique-looking utilities.
    let workspace = executive.parent().unwrap().parent().unwrap();
    assert!(workspace.join("crates/kernel/src/runtime.rs").is_file());
    assert!(workspace
        .join("crates/kernel/src/admission/budget.rs")
        .is_file());
    assert!(workspace.join("crates/fabric/src/ipc/mailbox.rs").is_file());
    assert!(workspace
        .join("crates/agora/src/scratchpad/mod.rs")
        .is_file());
}
