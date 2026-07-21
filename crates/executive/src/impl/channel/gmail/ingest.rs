//! Bounded MIME/body/attachment ingestion for authenticated Gmail messages.

use crate::r#impl::artifact::{
    ArtifactMetadata, ArtifactRecord, ArtifactScanStatus, ArtifactStore,
};
use async_trait::async_trait;
use fabric::ExternalIdentityId;
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

const HARD_MAX_BODY: usize = 256 * 1_024;
const HARD_MAX_TOTAL: u64 = 64 * 1_048_576;

#[derive(Debug, Clone)]
pub struct GmailMimePart {
    pub part_id: String,
    pub mime_type: String,
    pub filename: Option<String>,
    pub declared_size: Option<u64>,
    pub inline_body: Option<Vec<u8>>,
    pub attachment_id: Option<String>,
    pub parts: Vec<GmailMimePart>,
}

#[derive(Debug, Clone)]
pub struct GmailIngestMessage {
    pub account_id: ExternalIdentityId,
    pub message_id: String,
    pub thread_id: String,
    pub source_timestamp_ms: i64,
    pub root: GmailMimePart,
}

#[derive(Debug, Clone)]
pub struct GmailIngestConfig {
    pub max_depth: usize,
    pub max_parts: usize,
    pub max_attachments: usize,
    pub max_attachment_bytes: u64,
    pub max_total_bytes: u64,
    pub max_body_bytes: usize,
    pub allowed_attachment_mimes: HashSet<String>,
}

impl Default for GmailIngestConfig {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_parts: 100,
            max_attachments: 10,
            max_attachment_bytes: 8 * 1_048_576,
            max_total_bytes: 25 * 1_048_576,
            max_body_bytes: HARD_MAX_BODY,
            allowed_attachment_mimes: HashSet::from([
                "text/plain".into(),
                "application/pdf".into(),
                "image/png".into(),
                "image/jpeg".into(),
            ]),
        }
    }
}

#[async_trait]
pub trait GmailAttachmentFetcher: Send + Sync {
    async fn next_chunk(
        &self,
        attachment_id: &str,
        offset: u64,
        max_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Option<Vec<u8>>, String>;
}

#[derive(Debug, Clone)]
pub struct GmailOriginalReference {
    pub account_id: ExternalIdentityId,
    pub message_id: String,
    pub thread_id: String,
    pub source_timestamp_ms: i64,
}

#[derive(Debug, Clone)]
pub struct IngestedAttachment {
    pub part_id: String,
    pub filename: String,
    pub mime_type: String,
    pub artifact: Option<ArtifactRecord>,
    pub unavailable_reason: Option<String>,
}

impl IngestedAttachment {
    pub fn available_to_model(&self) -> bool {
        self.artifact
            .as_ref()
            .is_some_and(|artifact| artifact.scan_status == ArtifactScanStatus::Clean)
    }
}

#[derive(Debug, Clone)]
pub struct GmailIngestResult {
    pub body_text: String,
    pub original: GmailOriginalReference,
    pub attachments: Vec<IngestedAttachment>,
}

pub struct GmailMessageIngester {
    config: GmailIngestConfig,
}

impl GmailMessageIngester {
    pub fn new(config: GmailIngestConfig) -> anyhow::Result<Self> {
        validate_config(&config)?;
        Ok(Self { config })
    }

    pub async fn ingest(
        &self,
        message: &GmailIngestMessage,
        fetcher: &dyn GmailAttachmentFetcher,
        artifacts: &ArtifactStore,
        now_ms: i64,
        cancel: &CancellationToken,
    ) -> anyhow::Result<GmailIngestResult> {
        validate_message(message, now_ms)?;
        let mut flattened = Vec::new();
        flatten(&message.root, 0, &self.config, &mut flattened)?;
        let attachments: Vec<&GmailMimePart> = flattened
            .iter()
            .copied()
            .filter(|part| is_attachment(part))
            .collect();
        anyhow::ensure!(
            attachments.len() <= self.config.max_attachments,
            "attachment count exceeds cap"
        );
        let attachment_total = attachments.iter().try_fold(0_u64, |total, part| {
            let size = part.declared_size.unwrap_or(0);
            total
                .checked_add(size)
                .ok_or_else(|| anyhow::anyhow!("declared attachment size overflow"))
        })?;
        let inline_total = flattened.iter().try_fold(0_u64, |total, part| {
            total
                .checked_add(
                    part.inline_body
                        .as_ref()
                        .map_or(0, |body| body.len() as u64),
                )
                .ok_or_else(|| anyhow::anyhow!("inline body size overflow"))
        })?;
        anyhow::ensure!(
            attachment_total.saturating_add(inline_total) <= self.config.max_total_bytes,
            "message byte cap exceeded"
        );

        let body_text = extract_body(&flattened, self.config.max_body_bytes)?;
        let mut ingested = Vec::with_capacity(attachments.len());
        let mut seen_parts = HashSet::new();
        for part in attachments {
            if !seen_parts.insert(part.part_id.clone()) {
                ingested.push(unavailable(part, "duplicate_attachment"));
                continue;
            }
            if let Some(reason) = attachment_rejection(part, &self.config) {
                ingested.push(unavailable(part, reason));
                continue;
            }
            let declared = part.declared_size.expect("checked by rejection");
            let attachment_id = part.attachment_id.as_deref().expect("checked by rejection");
            let mut writer = artifacts.begin(
                ArtifactMetadata {
                    mime_type: part.mime_type.clone(),
                    provider: "google".into(),
                    account_id: message.account_id.to_string(),
                    provider_message_id: message.message_id.clone(),
                    provider_part_id: part.part_id.clone(),
                    source_timestamp_ms: message.source_timestamp_ms,
                    scan_status: ArtifactScanStatus::Unscanned,
                    created_at_ms: now_ms,
                },
                self.config.max_attachment_bytes,
            )?;
            let mut prefix = Vec::new();
            loop {
                if cancel.is_cancelled() {
                    anyhow::bail!("attachment ingestion cancelled");
                }
                let remaining = declared.saturating_sub(writer.size());
                let request = usize::try_from(remaining.min(64 * 1024)).unwrap_or(64 * 1024);
                let Some(chunk) = fetcher
                    .next_chunk(attachment_id, writer.size(), request.max(1), cancel)
                    .await
                    .map_err(anyhow::Error::msg)?
                else {
                    break;
                };
                anyhow::ensure!(!chunk.is_empty(), "attachment stream returned empty chunk");
                if prefix.len() < 512 {
                    prefix.extend_from_slice(&chunk[..chunk.len().min(512 - prefix.len())]);
                }
                writer.write_chunk(&chunk)?;
                anyhow::ensure!(
                    writer.size() <= declared,
                    "attachment exceeded declared size"
                );
            }
            anyhow::ensure!(writer.size() == declared, "partial attachment download");
            anyhow::ensure!(
                matches_declared_type(&part.mime_type, &prefix),
                "declared and actual MIME mismatch"
            );
            let artifact = artifacts.finish(writer)?;
            ingested.push(IngestedAttachment {
                part_id: part.part_id.clone(),
                filename: part.filename.clone().unwrap_or_else(|| "attachment".into()),
                mime_type: part.mime_type.clone(),
                artifact: Some(artifact),
                unavailable_reason: Some("unscanned".into()),
            });
        }
        Ok(GmailIngestResult {
            body_text,
            original: GmailOriginalReference {
                account_id: message.account_id,
                message_id: message.message_id.clone(),
                thread_id: message.thread_id.clone(),
                source_timestamp_ms: message.source_timestamp_ms,
            },
            attachments: ingested,
        })
    }
}

fn validate_config(config: &GmailIngestConfig) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1..=16).contains(&config.max_depth),
        "invalid MIME depth cap"
    );
    anyhow::ensure!(
        (1..=500).contains(&config.max_parts),
        "invalid MIME part cap"
    );
    anyhow::ensure!(
        (1..=50).contains(&config.max_attachments),
        "invalid attachment cap"
    );
    anyhow::ensure!(
        (1..=HARD_MAX_TOTAL).contains(&config.max_attachment_bytes),
        "invalid attachment byte cap"
    );
    anyhow::ensure!(
        config.max_attachment_bytes <= config.max_total_bytes
            && config.max_total_bytes <= HARD_MAX_TOTAL,
        "invalid total byte cap"
    );
    anyhow::ensure!(
        (1..=HARD_MAX_BODY).contains(&config.max_body_bytes),
        "invalid body cap"
    );
    Ok(())
}

fn validate_message(message: &GmailIngestMessage, now_ms: i64) -> anyhow::Result<()> {
    anyhow::ensure!(
        now_ms >= 0 && message.source_timestamp_ms >= 0,
        "invalid message time"
    );
    for value in [&message.message_id, &message.thread_id] {
        anyhow::ensure!(
            !value.is_empty() && value.len() <= 1_024,
            "invalid provider ID"
        );
    }
    Ok(())
}

fn flatten<'a>(
    part: &'a GmailMimePart,
    depth: usize,
    config: &GmailIngestConfig,
    output: &mut Vec<&'a GmailMimePart>,
) -> anyhow::Result<()> {
    anyhow::ensure!(depth <= config.max_depth, "nested multipart depth exceeded");
    anyhow::ensure!(
        !part.part_id.is_empty() && part.part_id.len() <= 1_024,
        "invalid MIME part ID"
    );
    anyhow::ensure!(valid_mime(&part.mime_type), "malformed MIME type");
    anyhow::ensure!(output.len() < config.max_parts, "MIME part count exceeded");
    if !part.parts.is_empty() {
        anyhow::ensure!(
            part.mime_type.starts_with("multipart/"),
            "children on non-multipart part"
        );
        anyhow::ensure!(
            part.inline_body.is_none() && part.attachment_id.is_none(),
            "multipart body is malformed"
        );
    }
    output.push(part);
    for child in &part.parts {
        flatten(child, depth + 1, config, output)?;
    }
    Ok(())
}

fn is_attachment(part: &GmailMimePart) -> bool {
    part.filename.is_some() || part.attachment_id.is_some()
}

fn attachment_rejection<'a>(
    part: &'a GmailMimePart,
    config: &GmailIngestConfig,
) -> Option<&'a str> {
    let filename = match part.filename.as_deref() {
        Some(filename) => filename,
        None => return Some("missing_filename"),
    };
    if filename.is_empty()
        || filename.contains(['/', '\\', '\0'])
        || filename == "."
        || filename == ".."
        || filename.contains("..")
    {
        return Some("unsafe_filename");
    }
    let lower = filename.to_ascii_lowercase();
    if [
        ".zip", ".rar", ".7z", ".tar", ".gz", ".exe", ".dll", ".js", ".sh", ".bat", ".cmd",
        ".docm", ".xlsm", ".pptm",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
    {
        return Some("dangerous_file_type");
    }
    if !config.allowed_attachment_mimes.contains(&part.mime_type) {
        return Some("mime_not_allowed");
    }
    let size = match part.declared_size {
        Some(size) => size,
        None => return Some("unknown_length"),
    };
    if size == 0 || size > config.max_attachment_bytes {
        return Some("attachment_size_rejected");
    }
    if part.attachment_id.as_deref().is_none_or(str::is_empty) {
        return Some("missing_attachment_id");
    }
    None
}

fn unavailable(part: &GmailMimePart, reason: &str) -> IngestedAttachment {
    IngestedAttachment {
        part_id: part.part_id.clone(),
        filename: part.filename.clone().unwrap_or_else(|| "attachment".into()),
        mime_type: part.mime_type.clone(),
        artifact: None,
        unavailable_reason: Some(reason.into()),
    }
}

fn extract_body(parts: &[&GmailMimePart], max_bytes: usize) -> anyhow::Result<String> {
    let plain = parts.iter().find(|part| {
        part.mime_type == "text/plain" && !is_attachment(part) && part.inline_body.is_some()
    });
    let html = parts.iter().find(|part| {
        part.mime_type == "text/html" && !is_attachment(part) && part.inline_body.is_some()
    });
    let (bytes, is_html) = plain
        .map(|part| (part.inline_body.as_deref().unwrap(), false))
        .or_else(|| html.map(|part| (part.inline_body.as_deref().unwrap(), true)))
        .unwrap_or((&[], false));
    anyhow::ensure!(bytes.len() <= max_bytes, "mail body exceeds cap");
    let text = std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("mail body is not UTF-8"))?;
    let text = if is_html {
        sanitize_html(text)
    } else {
        text.to_owned()
    };
    Ok(strip_history_and_signature(&text)
        .chars()
        .take(max_bytes)
        .collect())
}

fn sanitize_html(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut output = String::new();
    let mut index = 0;
    while index < input.len() {
        if lower[index..].starts_with("<script") || lower[index..].starts_with("<style") {
            let close = if lower[index..].starts_with("<script") {
                "</script>"
            } else {
                "</style>"
            };
            if let Some(end) = lower[index..].find(close) {
                index += end + close.len();
                continue;
            }
            break;
        }
        if input.as_bytes()[index] == b'<' {
            if let Some(end) = input[index..].find('>') {
                output.push(' ');
                index += end + 1;
                continue;
            }
            break;
        }
        let character = input[index..].chars().next().unwrap();
        output.push(character);
        index += character.len_utf8();
    }
    output
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn strip_history_and_signature(input: &str) -> String {
    let mut output = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed == "--"
            || trimmed == "-- "
            || (trimmed.starts_with("On ") && trimmed.ends_with(" wrote:"))
        {
            break;
        }
        if trimmed.starts_with('>') {
            continue;
        }
        output.push(line);
    }
    output.join("\n").trim().to_owned()
}

fn valid_mime(value: &str) -> bool {
    value.len() <= 256
        && value == value.to_ascii_lowercase()
        && value.split_once('/').is_some_and(|(kind, subtype)| {
            !kind.is_empty()
                && !subtype.is_empty()
                && kind
                    .bytes()
                    .chain(subtype.bytes())
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-'))
        })
}

fn matches_declared_type(mime: &str, prefix: &[u8]) -> bool {
    match mime {
        "application/pdf" => prefix.starts_with(b"%PDF-"),
        "image/png" => prefix.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => prefix.starts_with(&[0xff, 0xd8, 0xff]),
        "text/plain" => !prefix.contains(&0) && std::str::from_utf8(prefix).is_ok(),
        _ => false,
    }
}
