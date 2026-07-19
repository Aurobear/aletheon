//! Bounded newline-delimited JSON framing for ACP stdio/socket transports.

use std::io;

use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const DEFAULT_MAX_FRAME_BYTES: usize = 1024 * 1024;

pub struct AcpTransport<R, W> {
    reader: R,
    writer: W,
    max_frame_bytes: usize,
}

impl<R, W> AcpTransport<R, W>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
        }
    }

    pub fn with_max_frame_bytes(reader: R, writer: W, max_frame_bytes: usize) -> io::Result<Self> {
        if max_frame_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ACP frame limit must be non-zero",
            ));
        }
        Ok(Self {
            reader,
            writer,
            max_frame_bytes,
        })
    }

    /// Read one frame. EOF before any bytes returns `Ok(None)`.
    pub async fn read_frame<T: DeserializeOwned>(&mut self) -> io::Result<Option<T>> {
        let mut frame = Vec::new();
        let read = (&mut self.reader)
            .take((self.max_frame_bytes + 1) as u64)
            .read_until(b'\n', &mut frame)
            .await?;
        if read == 0 {
            return Ok(None);
        }
        if frame.len() > self.max_frame_bytes || !frame.ends_with(b"\n") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ACP frame exceeds limit or is not newline terminated",
            ));
        }
        frame.pop();
        if frame.last() == Some(&b'\r') {
            frame.pop();
        }
        serde_json::from_slice(&frame)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub async fn write_frame<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        let frame = serde_json::to_vec(value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if frame.len() + 1 > self.max_frame_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ACP frame exceeds limit",
            ));
        }
        self.writer.write_all(&frame).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await
    }

    pub fn into_inner(self) -> (R, W) {
        (self.reader, self.writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tokio::io::{duplex, BufReader};

    #[tokio::test]
    async fn reads_and_writes_newline_delimited_frames() {
        let (input_writer, input_reader) = duplex(128);
        let (output_writer, output_reader) = duplex(128);
        let mut transport = AcpTransport::new(BufReader::new(input_reader), output_writer);

        tokio::spawn(async move {
            let mut input_writer = input_writer;
            input_writer
                .write_all(b"{\"method\":\"cancel\"}\n")
                .await
                .unwrap();
        });
        let frame: Value = transport.read_frame().await.unwrap().unwrap();
        assert_eq!(frame["method"], "cancel");

        transport
            .write_frame(&json!({"result": "cancelled"}))
            .await
            .unwrap();
        let mut output_reader = BufReader::new(output_reader);
        let mut line = String::new();
        output_reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "{\"result\":\"cancelled\"}\n");
    }

    #[tokio::test]
    async fn rejects_oversize_and_unterminated_frames() {
        let (mut input_writer, input_reader) = duplex(128);
        let (output_writer, _output_reader) = duplex(128);
        let mut transport =
            AcpTransport::with_max_frame_bytes(BufReader::new(input_reader), output_writer, 8)
                .unwrap();
        input_writer.write_all(b"123456789").await.unwrap();
        input_writer.shutdown().await.unwrap();
        let error = transport.read_frame::<Value>().await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(transport.write_frame(&json!({"long": true})).await.is_err());
    }
}
