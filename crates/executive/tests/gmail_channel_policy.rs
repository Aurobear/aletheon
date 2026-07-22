use corpus::tools::google::oauth::GoogleBinding;
use executive::r#impl::channel::gmail::classifier::GmailClassification;
use executive::r#impl::channel::gmail::sender_policy::{
    AuthenticationRequirement, GmailHeader, GmailSenderPolicy, SenderPolicyError,
};
use executive::r#impl::channel::gmail::{
    GmailChannelMessage, GmailChannelStore, GmailInsertOutcome,
};
use executive::r#impl::external::ExternalIdentityRepository;
use fabric::{ExternalCapabilityId, ExternalIdentityId, PrincipalId};
use std::collections::HashSet;

struct Fixture {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    account: ExternalIdentityId,
    principal: PrincipalId,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objectives.db");
        let account = ExternalIdentityId::new();
        let principal = PrincipalId("owner".into());
        ExternalIdentityRepository::open(&path)
            .unwrap()
            .bind_google(
                &principal,
                GoogleBinding {
                    identity_id: account,
                    provider_subject: "subject".into(),
                    email: "owner@example.com".into(),
                    scopes: vec![ExternalCapabilityId::new("mail.read").unwrap()],
                },
                Some("work".into()),
                1,
            )
            .unwrap();
        Self {
            _dir: dir,
            path,
            account,
            principal,
        }
    }

    fn policy(&self, version: u64) -> GmailSenderPolicy {
        GmailSenderPolicy {
            principal: self.principal.clone(),
            version,
            allowed_addresses: HashSet::from(["alice@example.com".into()]),
            allowed_domains: HashSet::from(["trusted.example".into()]),
            trusted_authserv_ids: HashSet::from(["mx.google.com".into()]),
            authentication: AuthenticationRequirement::SpfOrDkim,
        }
    }

    fn message(&self, id: &str, from: &str, subject: &str) -> GmailChannelMessage {
        GmailChannelMessage {
            account_id: self.account,
            message_id: id.into(),
            thread_id: format!("thread-{id}"),
            headers: vec![
                header("From", from),
                header("Subject", subject),
                header(
                    "Authentication-Results",
                    "mx.google.com; spf=pass smtp.mailfrom=alice@example.com",
                ),
            ],
        }
    }
}

fn header(name: &str, value: &str) -> GmailHeader {
    GmailHeader {
        name: name.into(),
        value: value.into(),
    }
}

#[test]
fn verified_sender_exact_prefix_and_durable_evidence_are_persisted() {
    let fixture = Fixture::new();
    let store = GmailChannelStore::open(&fixture.path).unwrap();
    for (index, subject, expected) in [
        (1, "[ASK] question", GmailClassification::Ask),
        (2, "[GOAL] ship", GmailClassification::Goal),
        (3, "[MEMORY] fact", GmailClassification::Memory),
        (4, "[DOC] report", GmailClassification::Doc),
        (5, "[goal] wrong case", GmailClassification::Notification),
        (6, "[GOAL]suffix", GmailClassification::Notification),
        (7, "hello", GmailClassification::Notification),
    ] {
        let message = fixture.message(&format!("m{index}"), "Alice <alice@example.com>", subject);
        let (outcome, record) = store
            .authenticate_and_persist(&message, Some(&fixture.policy(7)), 1_000 + index)
            .unwrap();
        assert_eq!(outcome, GmailInsertOutcome::Inserted);
        assert_eq!(record.classification, expected);
        assert_eq!(record.verified_principal, Some(fixture.principal.clone()));
        assert_eq!(record.sender_policy_version, Some(7));
        assert_eq!(record.message_id, format!("m{index}"));
        assert_eq!(record.thread_id, format!("thread-m{index}"));
        assert_eq!(record.evidence_hash.len(), 64);
    }
}

#[test]
fn spoofing_forwarding_unicode_and_ambiguous_from_fail_closed() {
    let fixture = Fixture::new();
    let policy = fixture.policy(1);
    let mut spoof = fixture.message("spoof", "attacker@example.net", "[GOAL] attack");
    spoof.headers.push(header("Reply-To", "alice@example.com"));
    assert_eq!(
        policy.verify(&spoof.headers),
        Err(SenderPolicyError::Denied)
    );

    let unicode = fixture.message("unicode", "alice@exampлe.com", "[GOAL] attack");
    assert_eq!(
        policy.verify(&unicode.headers),
        Err(SenderPolicyError::AmbiguousFrom)
    );

    let mut multiple = fixture.message("multiple", "alice@example.com", "[GOAL] attack");
    multiple
        .headers
        .push(header("From", "attacker@example.net"));
    assert_eq!(
        policy.verify(&multiple.headers),
        Err(SenderPolicyError::AmbiguousFrom)
    );

    // Forwarded body/display text never participates in header identity proof.
    let forwarded = fixture.message(
        "forwarded",
        "Attacker saying From alice@example.com <attacker@example.net>",
        "[GOAL] forwarded",
    );
    assert_eq!(
        policy.verify(&forwarded.headers),
        Err(SenderPolicyError::Denied)
    );
}

#[test]
fn trusted_receiving_chain_alignment_and_authentication_modes_are_enforced() {
    let fixture = Fixture::new();
    let policy = fixture.policy(1);
    let mut injected = fixture.message("injected", "alice@example.com", "[GOAL] attack");
    injected.headers.insert(
        2,
        header(
            "Authentication-Results",
            "attacker.example; spf=pass smtp.mailfrom=alice@example.com",
        ),
    );
    assert_eq!(
        policy.verify(&injected.headers),
        Err(SenderPolicyError::UntrustedAuthenticationResults)
    );

    let mut duplicate_trusted = fixture.message("dup", "alice@example.com", "[GOAL] attack");
    duplicate_trusted.headers.push(header(
        "Authentication-Results",
        "mx.google.com; dkim=pass header.d=example.com",
    ));
    assert_eq!(
        policy.verify(&duplicate_trusted.headers),
        Err(SenderPolicyError::UntrustedAuthenticationResults)
    );

    let mut failed = fixture.message("failed", "alice@example.com", "[GOAL] attack");
    failed.headers[2].value =
        "mx.google.com; spf=fail smtp.mailfrom=alice@example.com; dkim=fail header.d=example.com"
            .into();
    assert_eq!(
        policy.verify(&failed.headers),
        Err(SenderPolicyError::AuthenticationFailed)
    );

    let mut dkim_policy = fixture.policy(2);
    dkim_policy.authentication = AuthenticationRequirement::Dkim;
    let mut dkim = fixture.message("dkim", "alice@example.com", "[GOAL] accepted");
    dkim.headers[2].value = "mx.google.com; dkim=pass header.d=example.com".into();
    assert!(dkim_policy.verify(&dkim.headers).is_ok());
    dkim_policy.authentication = AuthenticationRequirement::SpfAndDkim;
    assert_eq!(
        dkim_policy.verify(&dkim.headers),
        Err(SenderPolicyError::AuthenticationFailed)
    );
}

#[test]
fn policy_changes_duplicates_oversized_headers_and_default_deny_are_durable() {
    let fixture = Fixture::new();
    let store = GmailChannelStore::open(&fixture.path).unwrap();
    let message = fixture.message("same", "alice@example.com", "[GOAL] first");
    let (_, original) = store
        .authenticate_and_persist(&message, Some(&fixture.policy(1)), 1_000)
        .unwrap();

    let mut denied_policy = fixture.policy(2);
    denied_policy.allowed_addresses.clear();
    denied_policy.allowed_domains.clear();
    let (duplicate, retained) = store
        .authenticate_and_persist(&message, Some(&denied_policy), 2_000)
        .unwrap();
    assert_eq!(duplicate, GmailInsertOutcome::Duplicate);
    assert_eq!(retained, original);

    let changed = fixture.message("changed", "alice@example.com", "[GOAL] second");
    let (_, quarantined) = store
        .authenticate_and_persist(&changed, Some(&denied_policy), 2_001)
        .unwrap();
    assert_eq!(quarantined.classification, GmailClassification::Quarantine);
    assert!(quarantined.verified_principal.is_none());

    let default_denied = fixture.message("default", "alice@example.com", "[GOAL] third");
    let (_, quarantined) = store
        .authenticate_and_persist(&default_denied, None, 2_002)
        .unwrap();
    assert_eq!(quarantined.status, "quarantined");

    let mut wrong_principal_policy = fixture.policy(3);
    wrong_principal_policy.principal = PrincipalId("other".into());
    let wrong_principal = fixture.message("wrong-owner", "alice@example.com", "[GOAL] fourth");
    let (_, quarantined) = store
        .authenticate_and_persist(&wrong_principal, Some(&wrong_principal_policy), 2_003)
        .unwrap();
    assert!(quarantined.verified_principal.is_none());

    let mut oversized = fixture.message("oversized", "alice@example.com", "[GOAL] huge");
    oversized
        .headers
        .push(header("X-Huge", &"x".repeat(16 * 1024 + 1)));
    assert_eq!(
        fixture.policy(1).verify(&oversized.headers),
        Err(SenderPolicyError::MalformedHeaders)
    );
}
