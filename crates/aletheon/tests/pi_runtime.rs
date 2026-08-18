//! E6 regression coverage for the resident Pi delegate's public contract.

#[test]
fn pi_manifest_uses_the_canonical_delegate_id() {
    let manifest = aletheon::host::runtime::pi_manifest();
    assert_eq!(manifest.id, "pi-coder");
    assert!(manifest.aliases.iter().any(|alias| alias == "pi"));
}
