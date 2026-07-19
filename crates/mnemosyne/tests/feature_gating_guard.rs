//! Option A: cognitive backends are demoted behind off-by-default
//! `cognitive-memory` feature; EpisodicMemory stays default for the daemon.

#[test]
fn cognitive_exports_are_feature_gated() {
    let lib = include_str!("../src/lib.rs");
    assert!(
        lib.contains(r#"#[cfg(feature = "cognitive-memory")]"#),
        "cognitive re-exports must be gated behind the cognitive-memory feature"
    );
}

#[test]
fn episodic_memory_is_available_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let clock = std::sync::Arc::new(kernel::chronos::TestClock::default());
    let _mem = mnemosyne::EpisodicMemory::new(dir.path().join("ep.db"), clock);
}

#[cfg(feature = "cognitive-memory")]
#[test]
fn router_is_available_with_the_feature() {
    let dir = tempfile::tempdir().unwrap();
    let clock = std::sync::Arc::new(kernel::chronos::TestClock::default());
    let _router = mnemosyne::MemoryRouter::new(dir.path(), clock);
}
