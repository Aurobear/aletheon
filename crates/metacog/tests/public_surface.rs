use metacog::{DefaultMetaRuntime, EvaluationResult, EvolutionConfig, GenomeMeta};

#[test]
fn legacy_public_surface_remains_available_during_layout_migration() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<DefaultMetaRuntime>();
    let config = EvolutionConfig::default();
    assert!(!config.auto_evolve);
    let _evaluation = EvaluationResult {
        passed: true,
        reasons: Vec::new(),
    };
    let _: Option<GenomeMeta> = None;
}
