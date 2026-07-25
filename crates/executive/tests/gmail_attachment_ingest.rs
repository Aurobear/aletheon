use async_trait::async_trait;
use corpus::tools::google::oauth::GoogleBinding;
use executive::testing::artifact::ArtifactStore;
use executive::testing::channel::gmail::ingest::{
    ExternalEventIngestConfig, ExternalEventIngestMessage, GmailAttachmentFetcher,
    GmailMessageIngester, GmailMimePart,
};
use executive::testing::external::ExternalIdentityRepository;
use fabric::{ExternalCapabilityId, ExternalIdentityId, PrincipalId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

struct Fixture {
    _dir: tempfile::TempDir,
    db_path: std::path::PathBuf,
    artifact_root: std::path::PathBuf,
    account: ExternalIdentityId,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objectives.db");
        let artifact_root = dir.path().join("external-artifacts");
        let account = ExternalIdentityId::new();
        ExternalIdentityRepository::open(&db_path)
            .unwrap()
            .bind_google(
                &PrincipalId("owner".into()),
                GoogleBinding {
                    identity_id: account,
                    provider_subject: "subject".into(),
                    email: "owner@example.com".into(),
                    scopes: vec![ExternalCapabilityId::new("mail.read").unwrap()],
                },
                None,
                1,
            )
            .unwrap();
        Self {
            _dir: dir,
            db_path,
            artifact_root,
            account,
        }
    }

    fn artifacts(&self) -> ArtifactStore {
        ArtifactStore::open(&self.db_path, &self.artifact_root).unwrap()
    }

    fn message(&self, parts: Vec<GmailMimePart>) -> ExternalEventIngestMessage {
        ExternalEventIngestMessage {
            account_id: self.account,
            message_id: "message-1".into(),
            thread_id: "thread-1".into(),
            source_timestamp_ms: 1_000,
            root: GmailMimePart {
                part_id: "root".into(),
                mime_type: "multipart/mixed".into(),
                filename: None,
                declared_size: None,
                inline_body: None,
                attachment_id: None,
                parts,
            },
        }
    }
}

fn body(id: &str, mime: &str, value: &[u8]) -> GmailMimePart {
    GmailMimePart {
        part_id: id.into(),
        mime_type: mime.into(),
        filename: None,
        declared_size: Some(value.len() as u64),
        inline_body: Some(value.to_vec()),
        attachment_id: None,
        parts: Vec::new(),
    }
}

fn attachment(id: &str, filename: &str, mime: &str, size: Option<u64>) -> GmailMimePart {
    GmailMimePart {
        part_id: id.into(),
        mime_type: mime.into(),
        filename: Some(filename.into()),
        declared_size: size,
        inline_body: None,
        attachment_id: Some(format!("attachment-{id}")),
        parts: Vec::new(),
    }
}

struct Fetcher {
    values: HashMap<String, Vec<u8>>,
    chunk_size: usize,
    calls: AtomicUsize,
    cancel_after_first: Option<CancellationToken>,
}

#[async_trait]
impl GmailAttachmentFetcher for Fetcher {
    async fn next_chunk(
        &self,
        attachment_id: &str,
        offset: u64,
        max_bytes: usize,
        _cancel: &CancellationToken,
    ) -> Result<Option<Vec<u8>>, String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            if let Some(cancel) = &self.cancel_after_first {
                cancel.cancel();
            }
        }
        let bytes = self
            .values
            .get(attachment_id)
            .ok_or_else(|| "missing attachment".to_owned())?;
        let offset = usize::try_from(offset).map_err(|_| "bad offset")?;
        if offset >= bytes.len() {
            return Ok(None);
        }
        let end = (offset + self.chunk_size.min(max_bytes)).min(bytes.len());
        Ok(Some(bytes[offset..end].to_vec()))
    }
}

fn fetcher(values: impl IntoIterator<Item = (&'static str, Vec<u8>)>) -> Fetcher {
    Fetcher {
        values: values
            .into_iter()
            .map(|(id, value)| (id.to_owned(), value))
            .collect(),
        chunk_size: 3,
        calls: AtomicUsize::new(0),
        cancel_after_first: None,
    }
}

#[tokio::test]
async fn prefers_plain_strips_history_and_streams_unscanned_artifacts() {
    let fixture = Fixture::new();
    let pdf = b"%PDF-1.7 bounded".to_vec();
    let message = fixture.message(vec![
        body(
            "plain",
            "text/plain",
            b"Current request\n> quoted\nOn Tue wrote:\nold history",
        ),
        body(
            "html",
            "text/html",
            b"<p>wrong body</p><script>steal()</script>",
        ),
        attachment(
            "pdf",
            "report.pdf",
            "application/pdf",
            Some(pdf.len() as u64),
        ),
    ]);
    let fetcher = fetcher([("attachment-pdf", pdf)]);
    let artifacts = fixture.artifacts();
    let result = GmailMessageIngester::new(ExternalEventIngestConfig::default())
        .unwrap()
        .ingest(
            &message,
            &fetcher,
            &artifacts,
            2_000,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.body_text, "Current request");
    assert_eq!(result.original.message_id, "message-1");
    assert_eq!(result.attachments.len(), 1);
    let artifact = result.attachments[0].artifact.as_ref().unwrap();
    assert_eq!(artifact.size_bytes, 16);
    assert!(!result.attachments[0].available_to_model());
    assert!(artifacts.readable_path(artifact).unwrap().is_none());
}

#[tokio::test]
async fn sanitizes_html_and_lists_rejected_evidence_without_downloading() {
    let fixture = Fixture::new();
    let message = fixture.message(vec![
        body(
            "html",
            "text/html",
            b"<p>Hello</p><script src=x>secret</script><style>x{}</style><img onerror=evil>",
        ),
        attachment("path", "../secret.txt", "text/plain", Some(4)),
        attachment("archive", "payload.zip", "application/zip", Some(4)),
        attachment("unknown", "unknown.txt", "text/plain", None),
    ]);
    let fetcher = fetcher([]);
    let result = GmailMessageIngester::new(ExternalEventIngestConfig::default())
        .unwrap()
        .ingest(
            &message,
            &fetcher,
            &fixture.artifacts(),
            2_000,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(result.body_text.contains("Hello"));
    assert!(!result.body_text.contains("secret"));
    assert!(!result.body_text.contains("onerror"));
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        result
            .attachments
            .iter()
            .map(|attachment| attachment.unavailable_reason.as_deref().unwrap())
            .collect::<Vec<_>>(),
        vec!["unsafe_filename", "dangerous_file_type", "unknown_length"]
    );
}

#[tokio::test]
async fn malformed_bombs_partial_download_and_type_mismatch_fail_closed() {
    let fixture = Fixture::new();
    let mut malformed = fixture.message(vec![body("bad", "TEXT/PLAIN", b"bad")]);
    assert!(
        GmailMessageIngester::new(ExternalEventIngestConfig::default())
            .unwrap()
            .ingest(
                &malformed,
                &fetcher([]),
                &fixture.artifacts(),
                2_000,
                &CancellationToken::new(),
            )
            .await
            .is_err()
    );

    malformed.root.parts = vec![attachment(
        "huge",
        "huge.txt",
        "text/plain",
        Some(30 * 1_048_576),
    )];
    let no_download = fetcher([]);
    assert!(
        GmailMessageIngester::new(ExternalEventIngestConfig::default())
            .unwrap()
            .ingest(
                &malformed,
                &no_download,
                &fixture.artifacts(),
                2_000,
                &CancellationToken::new(),
            )
            .await
            .is_err()
    );
    assert_eq!(no_download.calls.load(Ordering::SeqCst), 0);

    let mut nested = body("leaf", "text/plain", b"leaf");
    for depth in 0..10 {
        nested = GmailMimePart {
            part_id: format!("nested-{depth}"),
            mime_type: "multipart/mixed".into(),
            filename: None,
            declared_size: None,
            inline_body: None,
            attachment_id: None,
            parts: vec![nested],
        };
    }
    let nested_message = ExternalEventIngestMessage {
        account_id: fixture.account,
        message_id: "nested".into(),
        thread_id: "nested-thread".into(),
        source_timestamp_ms: 1_000,
        root: nested,
    };
    assert!(
        GmailMessageIngester::new(ExternalEventIngestConfig::default())
            .unwrap()
            .ingest(
                &nested_message,
                &fetcher([]),
                &fixture.artifacts(),
                2_000,
                &CancellationToken::new(),
            )
            .await
            .is_err()
    );

    let partial = fixture.message(vec![attachment(
        "partial",
        "partial.txt",
        "text/plain",
        Some(10),
    )]);
    assert!(
        GmailMessageIngester::new(ExternalEventIngestConfig::default())
            .unwrap()
            .ingest(
                &partial,
                &fetcher([("attachment-partial", b"short".to_vec())]),
                &fixture.artifacts(),
                2_000,
                &CancellationToken::new(),
            )
            .await
            .is_err()
    );

    let mismatch = fixture.message(vec![attachment("png", "image.png", "image/png", Some(4))]);
    assert!(
        GmailMessageIngester::new(ExternalEventIngestConfig::default())
            .unwrap()
            .ingest(
                &mismatch,
                &fetcher([("attachment-png", b"text".to_vec())]),
                &fixture.artifacts(),
                2_000,
                &CancellationToken::new(),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn attachment_without_filename_is_rejected_without_panicking_or_downloading() {
    let fixture = Fixture::new();
    let message = fixture.message(vec![GmailMimePart {
        part_id: "nameless".into(),
        mime_type: "text/plain".into(),
        filename: None,
        declared_size: None,
        inline_body: None,
        attachment_id: Some("attachment-nameless".into()),
        parts: Vec::new(),
    }]);
    let no_download = fetcher([]);

    let result = GmailMessageIngester::new(ExternalEventIngestConfig::default())
        .unwrap()
        .ingest(
            &message,
            &no_download,
            &fixture.artifacts(),
            2_000,
            &CancellationToken::new(),
        )
        .await
        .expect("malformed attachment must be represented as rejected evidence");

    assert_eq!(no_download.calls.load(Ordering::SeqCst), 0);
    assert_eq!(result.attachments.len(), 1);
    assert_eq!(
        result.attachments[0].unavailable_reason.as_deref(),
        Some("missing_filename")
    );
}

#[tokio::test]
async fn duplicates_cancellation_and_restart_midway_leave_no_partial_evidence() {
    let fixture = Fixture::new();
    let message = fixture.message(vec![
        attachment("one", "one.txt", "text/plain", Some(4)),
        attachment("one", "duplicate.txt", "text/plain", Some(4)),
        attachment("two", "two.txt", "text/plain", Some(4)),
    ]);
    let data = b"data".to_vec();
    let result = GmailMessageIngester::new(ExternalEventIngestConfig::default())
        .unwrap()
        .ingest(
            &message,
            &fetcher([
                ("attachment-one", data.clone()),
                ("attachment-two", data.clone()),
            ]),
            &fixture.artifacts(),
            2_000,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.attachments[1].unavailable_reason.as_deref(),
        Some("duplicate_attachment")
    );
    assert_eq!(
        result.attachments[0].artifact.as_ref().unwrap().artifact_id,
        result.attachments[2].artifact.as_ref().unwrap().artifact_id
    );

    let cancel = CancellationToken::new();
    let cancelling = Fetcher {
        values: HashMap::from([("attachment-cancel".into(), b"cancel".to_vec())]),
        chunk_size: 2,
        calls: AtomicUsize::new(0),
        cancel_after_first: Some(cancel.clone()),
    };
    let cancel_message = fixture.message(vec![attachment(
        "cancel",
        "cancel.txt",
        "text/plain",
        Some(6),
    )]);
    assert!(
        GmailMessageIngester::new(ExternalEventIngestConfig::default())
            .unwrap()
            .ingest(
                &cancel_message,
                &cancelling,
                &fixture.artifacts(),
                3_000,
                &cancel,
            )
            .await
            .is_err()
    );
    assert!(!std::fs::read_dir(&fixture.artifact_root)
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".upload-")));

    std::fs::write(fixture.artifact_root.join(".upload-crashed"), b"partial").unwrap();
    drop(fixture.artifacts());
    assert!(!fixture.artifact_root.join(".upload-crashed").exists());
}
