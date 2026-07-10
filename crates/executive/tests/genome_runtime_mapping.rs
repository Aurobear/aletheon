#![allow(deprecated, clippy::field_reassign_with_default)]

//! Integration tests for Genome -> Runtime behavior mapping.

use executive::core::config::{GenomeConfig, RuntimeConfig};
use executive::core::orchestrator::AletheonRuntime;

#[test]
fn test_genome_config_default() {
    let config = GenomeConfig::default();
    assert_eq!(config.reasoning_strategy, "plan-then-execute");
    assert_eq!(config.impasse_threshold, 0.3);
    assert_eq!(config.genome_version, "0.1.0");
}

#[test]
fn test_runtime_holds_genome_config() {
    let mut runtime = AletheonRuntime::new(RuntimeConfig::default());
    let mut gc = GenomeConfig::default();
    gc.care_weights.insert("safety".to_string(), 1.0);
    runtime.update_genome_config(gc);
    assert_eq!(
        runtime.genome_config().care_weights.get("safety"),
        Some(&1.0)
    );
}

#[test]
fn test_genome_config_update_changes_strategy() {
    let mut runtime = AletheonRuntime::new(RuntimeConfig::default());
    assert_eq!(
        runtime.genome_config().reasoning_strategy,
        "plan-then-execute"
    );

    let mut gc = GenomeConfig::default();
    gc.reasoning_strategy = "direct".to_string();
    runtime.update_genome_config(gc);
    assert_eq!(runtime.genome_config().reasoning_strategy, "direct");
}

#[test]
fn test_care_weights_prompt_formatting() {
    let mut gc = GenomeConfig::default();
    gc.care_weights.insert("safety".to_string(), 0.9);
    gc.care_weights.insert("helpfulness".to_string(), 0.7);
    let prompt = gc.care_weights_prompt();
    assert!(prompt.contains("safety: 0.90"));
    assert!(prompt.contains("helpfulness: 0.70"));
    assert!(prompt.contains("Current care priorities"));
}

#[test]
fn test_care_weights_prompt_empty() {
    let gc = GenomeConfig::default();
    assert_eq!(gc.care_weights_prompt(), "");
}
