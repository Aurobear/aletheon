use super::config::CaptureConfig;

#[derive(Debug, Clone)]
pub struct CapturedOutput {
    pub content: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
}

pub fn capture_output(stdout: &[u8], stderr: &[u8], config: &CaptureConfig) -> CapturedOutput {
    let stdout_bytes = stdout.len();
    let stderr_bytes = stderr.len();

    let (stdout_capped, stdout_truncated) = cap_bytes(stdout, config.max_stdout_bytes);
    let (stderr_capped, stderr_truncated) = cap_bytes(stderr, config.max_stderr_bytes);

    let stdout_str = String::from_utf8_lossy(stdout_capped);
    let stderr_str = String::from_utf8_lossy(stderr_capped);

    let mut content = String::new();
    if !stdout_str.is_empty() {
        content.push_str(&stdout_str);
    }
    if !stderr_str.is_empty() {
        if !content.is_empty() {
            content.push_str("\n[stderr]\n");
        }
        content.push_str(&stderr_str);
    }
    if content.is_empty() {
        content = "(no output)".to_string();
    }

    CapturedOutput {
        content,
        stdout_truncated,
        stderr_truncated,
        stdout_bytes,
        stderr_bytes,
    }
}

fn cap_bytes(data: &[u8], max_bytes: usize) -> (&[u8], bool) {
    if data.len() <= max_bytes {
        return (data, false);
    }
    let mut end = max_bytes;
    while end > 0 && std::str::from_utf8(&data[..end]).is_err() {
        end -= 1;
    }
    (&data[..end], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_truncation_within_limits() {
        let config = CaptureConfig {
            max_stdout_bytes: 1000,
            max_stderr_bytes: 1000,
        };
        let result = capture_output(b"hello", b"world", &config);
        assert!(!result.stdout_truncated);
        assert!(!result.stderr_truncated);
        assert!(result.content.contains("hello"));
        assert!(result.content.contains("[stderr]"));
    }

    #[test]
    fn test_stdout_truncation() {
        let config = CaptureConfig {
            max_stdout_bytes: 10,
            max_stderr_bytes: 1000,
        };
        let stdout = b"this is a long stdout message";
        let result = capture_output(stdout, b"", &config);
        assert!(result.stdout_truncated);
        assert_eq!(result.stdout_bytes, 29);
    }

    #[test]
    fn test_empty_output() {
        let config = CaptureConfig::default();
        let result = capture_output(b"", b"", &config);
        assert_eq!(result.content, "(no output)");
    }

    #[test]
    fn test_stderr_only() {
        let config = CaptureConfig::default();
        let result = capture_output(b"", b"error message", &config);
        // When stdout is empty, stderr content is returned as-is (no prefix needed)
        assert_eq!(result.content, "error message");
        assert!(result.stderr_bytes > 0);
    }
}
