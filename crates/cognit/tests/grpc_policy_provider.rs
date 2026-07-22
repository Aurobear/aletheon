use cognit::policy::grpc_provider::{
    validate_policy_endpoint, GrpcPolicyConfig, StubPolicyProvider,
};
use cognit::ports::policy_provider::PolicyProviderPort;
use fabric::types::embodiment::{DeviceId, RiskClass, SkillDescriptor, SkillId};

#[test]
fn endpoint_validation_table() {
    let cases = vec![
        ("http://127.0.0.1:50052", true),
        ("http://localhost:50052", true),
        ("https://policy.example.com:443", true),
        ("grpcs://policy.internal:50052", true),
        ("http://policy-server:50052", false),
        ("", false),
    ];
    for (endpoint, expected) in cases {
        assert_eq!(
            validate_policy_endpoint(endpoint).is_ok(),
            expected,
            "endpoint: {}",
            endpoint
        );
    }
}

#[tokio::test]
async fn stub_produces_valid_proposal() {
    let provider = StubPolicyProvider::new(GrpcPolicyConfig::default());
    let skills = vec![SkillDescriptor {
        skill: SkillId("kuavo.stance".into()),
        device: DeviceId("bot".into()),
        summary: "s".into(),
        input_schema: serde_json::json!({}),
        risk: RiskClass::Low,
        timeout_ms: 10000,
        cancellable: false,
        preconditions: vec![],
        success_criteria: vec![],
    }];
    let proposals = provider
        .propose("goal", &DeviceId("bot".into()), &[], &[], &skills)
        .await
        .unwrap();
    assert!(!proposals.is_empty());
    // Verify proposal doesn't contain raw actuation
    assert!(!proposals[0]
        .parameters
        .as_object()
        .map_or(false, |p| p.contains_key("joint")));
}
