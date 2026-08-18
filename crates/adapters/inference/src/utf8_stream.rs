//! Lossless UTF-8 framing for provider byte streams.
//!
//! Network chunks are arbitrary byte ranges and may split a multi-byte UTF-8
//! scalar. Decoding each chunk with `from_utf8_lossy` permanently corrupts
//! otherwise valid SSE/NDJSON. Keep bytes until an ASCII framing delimiter is
//! present, then decode the complete frame strictly.

#[derive(Debug, Default)]
pub(super) struct Utf8StreamBuffer {
    bytes: Vec<u8>,
}

impl Utf8StreamBuffer {
    pub(super) fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
    }

    pub(super) fn take_line(&mut self) -> Result<Option<String>, std::string::FromUtf8Error> {
        self.take_frame(b"\n")
    }

    pub(super) fn take_event(&mut self) -> Result<Option<String>, std::string::FromUtf8Error> {
        self.take_frame(b"\n\n")
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.bytes.len()
    }

    fn take_frame(
        &mut self,
        delimiter: &[u8],
    ) -> Result<Option<String>, std::string::FromUtf8Error> {
        let Some(start) = self
            .bytes
            .windows(delimiter.len())
            .position(|window| window == delimiter)
        else {
            return Ok(None);
        };
        let remainder = self.bytes.split_off(start + delimiter.len());
        let mut frame = std::mem::replace(&mut self.bytes, remainder);
        frame.truncate(start);
        String::from_utf8(frame).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::Utf8StreamBuffer;

    #[test]
    fn arbitrary_network_chunks_preserve_multibyte_text() {
        let payload = "data: {\"text\":\"目标🙂完整\"}\n";
        let mut buffer = Utf8StreamBuffer::default();

        for byte in payload.as_bytes() {
            buffer.push(std::slice::from_ref(byte));
        }

        assert_eq!(
            buffer.take_line().unwrap().as_deref(),
            Some("data: {\"text\":\"目标🙂完整\"}")
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn invalid_complete_frame_is_rejected_instead_of_replaced() {
        let mut buffer = Utf8StreamBuffer::default();
        buffer.push(&[b'd', b'a', b't', b'a', b':', b' ', 0xff, b'\n']);

        assert!(buffer.take_line().is_err());
    }
}
