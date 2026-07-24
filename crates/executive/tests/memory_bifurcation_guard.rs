//! Locks in M-H Option A: the live runtime/daemon must not wire the
//! cognitive MemoryRouter. Source-scan guards so a future edit that
//! re-introduces the bifurcation fails CI.

#[test]
fn live_runtime_does_not_reference_cognitive_memory_router() {
    let orchestrator = include_str!("../src/core/orchestrator.rs");
    assert!(
        !orchestrator.contains("MemoryRouter"),
        "Option A: MemoryRouter must not be wired into AletheonExecutive"
    );
    assert!(
        !orchestrator.contains("with_memory"),
        "Option A: the never-called with_memory builder must be removed"
    );
}

#[test]
fn daemon_never_wires_a_memory_router_into_the_runtime() {
    let handler = include_str!("../src/host/daemon/handler/mod.rs");
    assert!(
        !handler.contains("with_memory("),
        "Option A: daemon must build AletheonExecutive without a MemoryRouter"
    );
    let memory_group = include_str!("../src/core/memory_group.rs");
    assert!(
        memory_group.contains("EpisodicMemory"),
        "EpisodicMemory remains the daemon's reflection store (grouped in MemoryGroup, Option A)"
    );
    let bootstrap = include_str!("../src/host/daemon/bootstrap/request.rs");
    assert!(
        bootstrap.contains("let memory_group = crate::core::MemoryGroup"),
        "MemoryGroup remains bootstrap-owned"
    );
}
