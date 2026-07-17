//! Model-visible shaping for tool output.

/// Maximum number of tool-result bytes retained in the active model context.
pub(super) const MAX_TOOL_RESULT_BYTES: usize = 8_000;

/// Return a UTF-8-safe head/tail view of an oversized tool result.
///
/// Callers must persist or emit the original result before using this transient
/// model-visible copy.
pub(super) fn bounded_tool_result(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_owned();
    }

    let marker = format!(
        "\n... [tool result truncated from {} bytes] ...\n",
        content.len()
    );
    if marker.len() >= max_bytes {
        return utf8_prefix(content, max_bytes).to_owned();
    }

    let payload_budget = max_bytes - marker.len();
    let head = utf8_prefix(content, payload_budget / 2);
    let tail = utf8_suffix(content, payload_budget - head.len());
    format!("{head}{marker}{tail}")
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    let mut boundary = max_bytes.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

fn utf8_suffix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = value.len() - max_bytes;
    while boundary < value.len() && !value.is_char_boundary(boundary) {
        boundary += 1;
    }
    &value[boundary..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_budget_is_unchanged() {
        assert_eq!(bounded_tool_result("还是A吧", 64), "还是A吧");
    }

    #[test]
    fn bounds_ascii_with_head_and_tail() {
        let input = format!("HEAD{}TAIL", "x".repeat(200));
        let bounded = bounded_tool_result(&input, 80);
        assert!(bounded.len() <= 80);
        assert!(bounded.starts_with("HEAD"));
        assert!(bounded.ends_with("TAIL"));
        assert!(bounded.contains("truncated from 208 bytes"));
    }

    #[test]
    fn bounds_chinese_on_utf8_boundaries() {
        let input = format!("开头{}结尾", "运控".repeat(100));
        let bounded = bounded_tool_result(&input, 96);
        assert!(bounded.len() <= 96);
        assert!(bounded.starts_with("开"));
        assert!(bounded.ends_with("结尾"));
        assert!(bounded.contains("bytes"));
    }

    #[test]
    fn bounds_emoji_on_utf8_boundaries() {
        let input = format!("start{}finish", "🧠".repeat(100));
        let bounded = bounded_tool_result(&input, 97);
        assert!(bounded.len() <= 97);
        assert!(bounded.starts_with("start"));
        assert!(bounded.ends_with("finish"));
        assert!(bounded.contains("bytes"));
    }
}
