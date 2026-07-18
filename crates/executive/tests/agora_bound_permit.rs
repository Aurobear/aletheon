#[test]
fn agora_bound_permit_is_the_production_commit_path() {
    let source = include_str!("../src/service/turn_pipeline.rs");
    assert!(source.contains("agora.commit(id, permit)"));
    assert!(!source.contains("agora.commit(&session_id_for_agora"));
}
