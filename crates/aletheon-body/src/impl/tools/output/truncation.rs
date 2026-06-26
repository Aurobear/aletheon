use super::config::TruncationPolicy;

#[derive(Debug, Clone)]
pub struct TruncatedContent {
    pub content: String,
    pub was_truncated: bool,
    pub original_bytes: usize,
}

pub fn truncate_head_tail(content: &str, policy: &TruncationPolicy) -> TruncatedContent {
    let original_bytes = content.len();
    let total_lines = content.lines().count();
    let max_lines = policy.head_lines + policy.tail_lines;

    if total_lines <= max_lines {
        if let Some(max_bytes) = policy.max_bytes {
            if original_bytes > max_bytes {
                return truncate_by_bytes(content, max_bytes, original_bytes);
            }
        }
        return TruncatedContent {
            content: content.to_string(),
            was_truncated: false,
            original_bytes,
        };
    }

    let lines: Vec<&str> = content.lines().collect();
    let head: String = lines[..policy.head_lines]
        .iter()
        .map(|l| format!("{}\n", l))
        .collect();
    let tail: String = lines[total_lines - policy.tail_lines..]
        .iter()
        .map(|l| format!("{}\n", l))
        .collect();
    let omitted = total_lines - policy.head_lines - policy.tail_lines;

    let truncated = format!("{}[... {} lines omitted ...]\n{}", head, omitted, tail);

    if let Some(max_bytes) = policy.max_bytes {
        if truncated.len() > max_bytes {
            return truncate_by_bytes(&truncated, max_bytes, original_bytes);
        }
    }

    TruncatedContent {
        content: truncated,
        was_truncated: true,
        original_bytes,
    }
}

fn truncate_by_bytes(content: &str, max_bytes: usize, original_bytes: usize) -> TruncatedContent {
    if content.len() <= max_bytes {
        return TruncatedContent {
            content: content.to_string(),
            was_truncated: false,
            original_bytes,
        };
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = format!(
        "{}\n[... truncated at {}/{} bytes ...]",
        &content[..end], end, original_bytes
    );
    TruncatedContent {
        content: truncated,
        was_truncated: true,
        original_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_truncation_when_within_limits() {
        let content = "line1\nline2\nline3\n";
        let policy = TruncationPolicy {
            head_lines: 5,
            tail_lines: 5,
            max_bytes: None,
        };
        let result = truncate_head_tail(content, &policy);
        assert!(!result.was_truncated);
    }

    #[test]
    fn test_head_tail_truncation() {
        let lines: Vec<String> = (0..100).map(|i| format!("line {}", i)).collect();
        let content = lines.join("\n");
        let policy = TruncationPolicy {
            head_lines: 3,
            tail_lines: 2,
            max_bytes: None,
        };
        let result = truncate_head_tail(&content, &policy);
        assert!(result.was_truncated);
        assert!(result.content.contains("line 0"));
        assert!(result.content.contains("line 99"));
        assert!(result.content.contains("[... 95 lines omitted ...]"));
    }

    #[test]
    fn test_utf8_safe_byte_truncation() {
        let content: String = "你好世界".repeat(1000);
        let policy = TruncationPolicy {
            head_lines: 100,
            tail_lines: 100,
            max_bytes: Some(100),
        };
        let result = truncate_head_tail(&content, &policy);
        assert!(result.was_truncated);
        assert!(std::str::from_utf8(result.content.as_bytes()).is_ok());
    }
}
