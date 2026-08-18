#[test]
fn agora_bound_permit_is_the_production_commit_path() {
    // The production agora commit is the permit-bound form.  During the
    // wiring-ownership migration (M7.4 host/runtime 归位) the turn-driven
    // evidence commit moved out of host/turn_pipeline.rs into the conscious
    // workspace adapter; the conscious-core coordinator commit is the second
    // permit-bound production site.  Both must stay permit-bound and must not
    // fall back to the session-id-based form.
    let evidence_commit = include_str!("../src/adapters/conscious/turn_workspace.rs");
    let core_commit = include_str!("../src/composition/conscious_core_coordinator.rs");
    assert!(
        evidence_commit.contains("agora.commit(id, permit)")
            || core_commit.contains("agora.commit(proposal_id, permit)")
    );
    // The old session-id-shaped commit must not reappear in the turn pipeline
    // or at either production commit site.
    let turn_pipeline = include_str!("../src/host/turn_pipeline.rs");
    for source in [&evidence_commit[..], &core_commit[..], &turn_pipeline[..]] {
        assert!(!source.contains("agora.commit(&session_id_for_agora"));
    }
}
