//! Locks in M-H Option A: the live runtime/daemon must not wire the
//! cognitive MemoryRouter. Source-scan guards so a future edit that
//! re-introduces the bifurcation fails CI.

#[test]
fn live_runtime_does_not_reference_cognitive_memory_router() {
    let orchestrator = include_str!("../src/core/orchestrator.rs");
    assert!(
        !orchestrator.contains("MemoryRouter"),
        "Option A: MemoryRouter must not be wired into AletheonRuntime"
    );
    assert!(
        !orchestrator.contains("with_memory"),
        "Option A: the never-called with_memory builder must be removed"
    );
}

#[test]
fn daemon_never_wires_a_memory_router_into_the_runtime() {
    let handler = include_str!("../src/impl/daemon/handler/mod.rs");
    assert!(
        !handler.contains("with_memory("),
        "Option A: daemon must build AletheonRuntime without a MemoryRouter"
    );
    assert!(
        handler.contains("EpisodicMemory"),
        "EpisodicMemory remains the daemon's reflection store (kept under Option A)"
    );
}
