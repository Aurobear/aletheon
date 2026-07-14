//! gbrain recall and capture integration tests.
//!
//! Tests the public API of the gbrain module:
//! - Recall skip rules for low-signal inputs
//! - Capture validation and slug stability
//! - Redaction of secrets
//! - Markdown serialization with correct frontmatter
//! - Outbox atomicity, idempotency, and replay lifecycle

use executive::service::daemon_turn::gbrain::{
    compute_slug, redact_secrets, GbrainCapture, GbrainOutbox, Provenance,
    should_skip_recall, capture_to_markdown,
};

// ---------------------------------------------------------------------------
// Recall gate tests
// ---------------------------------------------------------------------------

#[test]
fn recall_skips_empty_input() {
    assert!(should_skip_recall(""));
}

#[test]
fn recall_skips_slash_commands() {
    assert!(should_skip_recall("/help"));
    assert!(should_skip_recall("/status"));
}

#[test]
fn recall_skips_low_signal() {
    assert!(should_skip_recall("hi"));
    assert!(should_skip_recall("hello"));
    assert!(should_skip_recall("thanks"));
    assert!(should_skip_recall("ok"));
    assert!(should_skip_recall("好的"));
    assert!(should_skip_recall("谢谢"));
}

#[test]
fn recall_accepts_meaningful_questions() {
    assert!(!should_skip_recall("what was the last fix for EtherCAT jitter?"));
    assert!(!should_skip_recall("我们上次如何修复 EtherCAT 抖动？"));
}

// ---------------------------------------------------------------------------
// Capture validation tests
// ---------------------------------------------------------------------------

#[test]
fn capture_slug_is_stable_and_source_scoped() {
    let slug_a = compute_slug("aletheon", "workspace-a", "aletheon", "session-7");
    let slug_b = compute_slug("aletheon", "workspace-a", "aletheon", "session-7");
    assert_eq!(slug_a, slug_b, "slug must be deterministic");
    assert!(slug_a.starts_with("aletheon/sessions/"), "slug must be source-scoped");
}

#[test]
fn capture_different_sessions_different_slugs() {
    let s1 = compute_slug("aletheon", "ws", "aletheon", "session-a");
    let s2 = compute_slug("aletheon", "ws", "aletheon", "session-b");
    assert_ne!(s1, s2);
}

#[test]
fn capture_validate_rejects_empty_summary() {
    let capture = GbrainCapture {
        workspace: "test".into(),
        session_id: "s1".into(),
        summary: "".into(),
        provenance: vec![],
        confidence: 1.0,
    };
    assert!(GbrainOutbox::validate_capture(&capture).is_err());
}

#[test]
fn capture_validate_rejects_whitespace_summary() {
    let capture = GbrainCapture {
        workspace: "test".into(),
        session_id: "s1".into(),
        summary: "   \n  ".into(),
        provenance: vec![],
        confidence: 1.0,
    };
    assert!(GbrainOutbox::validate_capture(&capture).is_err());
}

#[test]
fn capture_validate_rejects_confidence_out_of_range() {
    let too_high = GbrainCapture {
        workspace: "test".into(),
        session_id: "s1".into(),
        summary: "valid".into(),
        provenance: vec![],
        confidence: 1.5,
    };
    assert!(GbrainOutbox::validate_capture(&too_high).is_err());

    let negative = GbrainCapture {
        workspace: "test".into(),
        session_id: "s1".into(),
        summary: "valid".into(),
        provenance: vec![],
        confidence: -0.5,
    };
    assert!(GbrainOutbox::validate_capture(&negative).is_err());
}

#[test]
fn capture_validate_rejects_empty_session_id() {
    let capture = GbrainCapture {
        workspace: "test".into(),
        session_id: "".into(),
        summary: "valid summary".into(),
        provenance: vec![],
        confidence: 1.0,
    };
    assert!(GbrainOutbox::validate_capture(&capture).is_err());
}

#[test]
fn capture_validate_rejects_empty_workspace() {
    let capture = GbrainCapture {
        workspace: "".into(),
        session_id: "s1".into(),
        summary: "valid summary".into(),
        provenance: vec![],
        confidence: 1.0,
    };
    assert!(GbrainOutbox::validate_capture(&capture).is_err());
}

#[test]
fn capture_validate_accepts_valid() {
    let capture = GbrainCapture {
        workspace: "aletheon".into(),
        session_id: "session-7".into(),
        summary: "Validated summary of the session".into(),
        provenance: vec![Provenance::RuntimeVerified],
        confidence: 0.9,
    };
    assert!(GbrainOutbox::validate_capture(&capture).is_ok());
}

// ---------------------------------------------------------------------------
// Redaction tests
// ---------------------------------------------------------------------------

#[test]
fn redact_bearer_tokens() {
    let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abc.def and more text";
    let result = redact_secrets(input);
    assert!(!result.contains("eyJhbGciOiJIUzI1NiJ9"), "JWT must be redacted");
    assert!(!result.contains("abc.def"), "bearer token must be redacted");
    assert!(result.contains("[REDACTED]"));
    assert!(result.contains("and more text"), "non-secret text preserved");
}

#[test]
fn redact_sk_api_keys() {
    let input = "API key: sk-1234567890abcdefghij more content";
    let result = redact_secrets(input);
    assert!(!result.contains("sk-1234567890abcdefghij"), "sk- key redacted");
    assert!(result.contains("[REDACTED]"));
    assert!(result.contains("more content"));
}

#[test]
fn redact_token_equals_pattern() {
    let input = "set token=my-secret-key-here please";
    let result = redact_secrets(input);
    assert!(!result.contains("my-secret-key-here"));
    assert!(result.contains("[REDACTED]"));
}

#[test]
fn redact_password_equals_pattern() {
    let input = "password=hunter2 and the rest";
    let result = redact_secrets(input);
    assert!(!result.contains("hunter2"));
    assert!(result.contains("[REDACTED]"));
}

#[test]
fn redact_secret_equals_pattern() {
    let input = "secret=classified-data here";
    let result = redact_secrets(input);
    assert!(!result.contains("classified-data"));
    assert!(result.contains("[REDACTED]"));
}

#[test]
fn redact_preserves_clean_text() {
    let input = "The EtherCAT fix was to increase cycle time to 2ms for V55 boards.";
    let result = redact_secrets(input);
    assert_eq!(result, input, "clean text must be unchanged");
}

#[test]
fn redact_multiple_patterns() {
    let input = "Auth: Bearer tok1 and key=sk-tok2 and token=val3";
    let result = redact_secrets(input);
    assert!(!result.contains("tok1"));
    assert!(!result.contains("sk-tok2"));
    assert!(!result.contains("val3"));
    assert_eq!(result.matches("[REDACTED]").count(), 3, "three redactions expected");
}

// ---------------------------------------------------------------------------
// Markdown serialization tests
// ---------------------------------------------------------------------------

#[test]
fn markdown_contains_provenance_fields() {
    let capture = GbrainCapture {
        workspace: "aletheon".into(),
        session_id: "session-7".into(),
        summary: "Ran benchmark and validated results.".into(),
        provenance: vec![Provenance::RuntimeVerified, Provenance::ToolDerived],
        confidence: 0.95,
    };
    let slug = compute_slug("aletheon", "aletheon", "aletheon", "session-7");
    let md = capture_to_markdown(&capture, &slug);

    assert!(md.contains("runtime_verified"), "provenance must appear");
    assert!(md.contains("tool_derived"), "all provenance tags must appear");
    assert!(md.contains("0.95"), "confidence must appear");
    assert!(md.contains(&slug), "slug must appear");
}

#[test]
fn markdown_redacts_secrets() {
    let capture = GbrainCapture {
        workspace: "aletheon".into(),
        session_id: "s1".into(),
        summary: "Used key=sk-abc123 and Bearer xyz to access API.".into(),
        provenance: vec![],
        confidence: 1.0,
    };
    let slug = compute_slug("aletheon", "aletheon", "aletheon", "s1");
    let md = capture_to_markdown(&capture, &slug);

    assert!(!md.contains("sk-abc123"), "sk- key must be redacted");
    assert!(!md.contains("xyz"), "bearer token must be redacted");
    assert!(md.contains("[REDACTED]"));
    assert!(md.contains("Used key="), "non-secret structure preserved");
}

#[test]
fn markdown_has_yaml_frontmatter() {
    let capture = GbrainCapture {
        workspace: "test".into(),
        session_id: "s42".into(),
        summary: "summary text".into(),
        provenance: vec![Provenance::UserConfirmed],
        confidence: 0.8,
    };
    let slug = compute_slug("test", "test", "test", "s42");
    let md = capture_to_markdown(&capture, &slug);

    // Verify three --- delimiters (start, end, end-of-file marker not needed)
    let dashes: Vec<&str> = md.lines().filter(|l| *l == "---").collect();
    assert_eq!(dashes.len(), 2, "must have opening and closing ---");

    // Field order: source, project, producer, session_id, slug, created_at,
    // provenance, confidence, sensitivity
    let lines: Vec<&str> = md.lines().collect();
    let first_dash = lines.iter().position(|l| *l == "---").unwrap();
    let second_dash = lines[first_dash + 1..].iter().position(|l| *l == "---").unwrap() + first_dash + 1;

    let fm = &lines[first_dash + 1..second_dash];
    assert!(fm[0].starts_with("source:"), "field 1: {}", fm[0]);
    assert!(fm[1].starts_with("project:"), "field 2: {}", fm[1]);
    assert!(fm[2].starts_with("producer:"), "field 3: {}", fm[2]);
    assert!(fm[3].starts_with("session_id:"), "field 4: {}", fm[3]);
    assert!(fm[4].starts_with("slug:"), "field 5: {}", fm[4]);
    assert!(fm[5].starts_with("created_at:"), "field 6: {}", fm[5]);

    // Body should appear after frontmatter
    let body_start = second_dash + 1;
    let body = lines[body_start..].join("\n");
    assert!(body.contains("summary text"), "body must appear after frontmatter: {}", body);
}

#[test]
fn markdown_without_provenance_omits_field() {
    let capture = GbrainCapture {
        workspace: "test".into(),
        session_id: "s1".into(),
        summary: "body".into(),
        provenance: vec![],
        confidence: 1.0,
    };
    let slug = compute_slug("test", "test", "test", "s1");
    let md = capture_to_markdown(&capture, &slug);

    // When provenance is empty, the field should not appear
    assert!(!md.contains("provenance:"), "empty provenance must not appear in frontmatter");
}

// ---------------------------------------------------------------------------
// Outbox integration tests
// ---------------------------------------------------------------------------

#[test]
fn outbox_enqueue_is_idempotent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let outbox = GbrainOutbox::new(&tmp.path().to_string_lossy());

    let slug = "test/sessions/2026-07-15-abcdef0123456789";
    let path1 = outbox.enqueue(slug, "content v1").unwrap();
    let path2 = outbox.enqueue(slug, "content v2").unwrap();
    assert_eq!(path1, path2, "enqueue must return same path on duplicate");

    // Content should be from the first write
    let data = std::fs::read_to_string(&path1).unwrap();
    assert!(data.contains("content v1"), "first write must win: {}", data);
    assert!(!data.contains("content v2"), "second write must not overwrite: {}", data);
}

#[test]
fn outbox_atomic_enqueue_permissions() {
    let tmp = tempfile::TempDir::new().unwrap();
    let outbox = GbrainOutbox::new(&tmp.path().to_string_lossy());

    let slug = "test/sessions/2026-07-15-deadbeef00000000";
    let path = outbox.enqueue(slug, "content").unwrap();
    assert!(path.exists());
    assert!(path.is_file());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "outbox file must be mode 0600, got {:o}", mode);
    }
}

#[test]
fn outbox_interrupted_tmp_skipped() {
    let tmp = tempfile::TempDir::new().unwrap();
    let outbox_dir = tmp.path().to_string_lossy();
    let outbox = GbrainOutbox::new(&outbox_dir);

    // Pre-create the directory
    std::fs::create_dir_all(tmp.path()).unwrap();

    // Simulate an interrupted write (only .tmp file, no .json)
    let tmp_file = tmp.path().join("abc1234567890def.tmp");
    std::fs::write(&tmp_file, b"partial write").unwrap();

    let pending = outbox.pending().unwrap();
    assert!(pending.is_empty(), ".tmp files must be skipped by pending()");
}

#[test]
fn outbox_replay_preserves_on_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let outbox = GbrainOutbox::new(&tmp.path().to_string_lossy());

    let slug = "test/sessions/2026-07-15-feedfacecafebabe";
    let path = outbox.enqueue(slug, "test markdown").unwrap();
    assert!(path.exists(), "entry must exist before replay");

    let rt = tokio::runtime::Runtime::new().unwrap();

    // Replay with failure
    let (sent, failed) = rt.block_on(outbox.replay(
        |_slug: String, _md: String| async { Err("remote down".to_string()) },
        10,
    ));
    assert_eq!(sent, 0);
    assert_eq!(failed, 1);
    assert!(path.exists(), "entry preserved after failure");

    // Verify entry was updated with failure metadata
    let pending = outbox.pending().unwrap();
    assert_eq!(pending.len(), 0, "entry should have backoff delay (not eligible yet)");
}

#[test]
fn outbox_replay_deletes_on_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let outbox = GbrainOutbox::new(&tmp.path().to_string_lossy());

    let slug = "test/sessions/2026-07-15-cafebabedeadbeef";
    let path = outbox.enqueue(slug, "test markdown").unwrap();
    assert!(path.exists());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (sent, failed) = rt.block_on(outbox.replay(
        |_slug: String, _md: String| async { Ok(()) },
        10,
    ));
    assert_eq!(sent, 1);
    assert_eq!(failed, 0);
    assert!(!path.exists(), "entry must be deleted after successful replay");
}

#[test]
fn outbox_replay_respects_limit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let outbox = GbrainOutbox::new(&tmp.path().to_string_lossy());

    // Enqueue 3 entries with different slugs
    outbox.enqueue("a/sessions/2026-01-01-0000000000000001", "a").unwrap();
    outbox.enqueue("a/sessions/2026-01-01-0000000000000002", "b").unwrap();
    outbox.enqueue("a/sessions/2026-01-01-0000000000000003", "c").unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (sent, _failed) = rt.block_on(outbox.replay(
        |_slug: String, _md: String| async { Ok(()) },
        2, // limit to 2
    ));
    assert_eq!(sent, 2, "replay must respect the limit");

    // One entry should remain pending
    let remaining = outbox.pending().unwrap();
    assert_eq!(remaining.len(), 1, "one entry should remain after limited replay");
}

#[test]
fn capture_to_markdown_secrets_redacted_in_body() {
    // Verify the full pipeline: capture -> markdown -> no secrets
    let capture = GbrainCapture {
        workspace: "aletheon".into(),
        session_id: "session-7".into(),
        summary: "We fixed the bug using token=sk-abcdef and Bearer xyz789.".into(),
        provenance: vec![Provenance::RuntimeVerified],
        confidence: 0.9,
    };
    let slug = compute_slug("aletheon", "aletheon", "aletheon", "session-7");
    let md = capture_to_markdown(&capture, &slug);

    // No secrets in output
    assert!(!md.contains("sk-abcdef"));
    assert!(!md.contains("xyz789"));
    assert!(md.contains("[REDACTED]"));

    // Provenance and slug present
    assert!(md.contains("runtime_verified"));
    assert!(md.contains(&slug));
    assert!(md.contains("0.9"));
}
