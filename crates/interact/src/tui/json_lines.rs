//! Lossless UTF-8 framing for newline-delimited daemon protocol messages.
//!
//! Unix socket reads may split a multi-byte scalar. Keep raw bytes until the
//! JSON-line delimiter arrives, then decode the complete frame strictly.

#[derive(Debug, Default)]
pub(crate) struct JsonLineBuffer {
    bytes: Vec<u8>,
}

impl JsonLineBuffer {
    pub(crate) fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
    }

    pub(crate) fn clear(&mut self) {
        self.bytes.clear();
    }

    pub(crate) fn take_line(&mut self) -> Result<Option<String>, std::string::FromUtf8Error> {
        let Some(newline) = self.bytes.iter().position(|byte| *byte == b'\n') else {
            return Ok(None);
        };
        let remainder = self.bytes.split_off(newline + 1);
        let mut line = std::mem::replace(&mut self.bytes, remainder);
        line.truncate(newline);
        String::from_utf8(line).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::JsonLineBuffer;

    #[test]
    fn preserves_multibyte_json_split_across_socket_reads() {
        let payload = "{\"text\":\"目标🙂完整\"}\n";
        let mut buffer = JsonLineBuffer::default();
        for byte in payload.as_bytes() {
            buffer.push(std::slice::from_ref(byte));
        }
        assert_eq!(buffer.take_line().unwrap().as_deref(), Some(payload.trim()));
    }

    #[test]
    fn rejects_invalid_complete_json_line_instead_of_replacing() {
        let mut buffer = JsonLineBuffer::default();
        buffer.push(&[b'{', b'"', b'x', b'"', b':', 0xff, b'}', b'\n']);
        assert!(buffer.take_line().is_err());
    }
}
