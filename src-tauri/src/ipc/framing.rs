//! LSP-style `Content-Length` JSON-RPC 2.0 framing (LLD-02 §4.2,
//! BACKEND_STANDARDS §2). Binary-safe UTF-8 framing on both directions of
//! the worker's stdio pipe.
//!
//! Plain functions over `AsyncBufRead`/`AsyncWrite` so the same code frames
//! real child-process pipes in production and in-memory buffers in tests.

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// No legitimate v1 message approaches this. Caps a runaway peer.
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FramingError {
    #[error("missing or duplicate Content-Length header")]
    MissingContentLength,
    #[error("malformed Content-Length header: {0}")]
    MalformedContentLength(String),
    #[error("Content-Length {0} exceeds the {MAX_BODY_BYTES} byte cap")]
    BodyTooLarge(usize),
    #[error("body shorter than declared Content-Length")]
    TruncatedBody,
    #[error("I/O error while framing: {0}")]
    Io(#[from] std::io::Error),
}

pub fn encode_frame(body: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    body: &[u8],
) -> Result<(), FramingError> {
    w.write_all(&encode_frame(body)).await?;
    w.flush().await?;
    Ok(())
}

/// Reads one frame. `Ok(None)` means clean EOF before any header byte
/// arrived (the peer closed its write end) — not an error.
pub async fn read_frame<R: AsyncBufRead + Unpin>(
    r: &mut R,
) -> Result<Option<Vec<u8>>, FramingError> {
    let mut content_length: Option<usize> = None;
    let mut saw_any_header_line = false;

    loop {
        let mut line = Vec::new();
        let n = r.read_until(b'\n', &mut line).await?;
        if n == 0 {
            if saw_any_header_line {
                return Err(FramingError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF mid-header",
                )));
            }
            return Ok(None);
        }
        while matches!(line.last(), Some(b'\n') | Some(b'\r')) {
            line.pop();
        }
        if line.is_empty() {
            break; // blank line ends the header block
        }
        saw_any_header_line = true;

        let text = String::from_utf8_lossy(&line);
        let (key, value) = text
            .split_once(':')
            .ok_or_else(|| FramingError::MalformedContentLength(text.to_string()))?;
        if key.trim().eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(FramingError::MissingContentLength); // duplicate header
            }
            let len: usize = value
                .trim()
                .parse()
                .map_err(|_| FramingError::MalformedContentLength(value.trim().to_string()))?;
            content_length = Some(len);
        }
        // Unknown headers are ignored (forwards-compat).
    }

    let length = content_length.ok_or(FramingError::MissingContentLength)?;
    if length > MAX_BODY_BYTES {
        return Err(FramingError::BodyTooLarge(length));
    }

    let mut body = vec![0u8; length];
    r.read_exact(&mut body)
        .await
        .map_err(|_| FramingError::TruncatedBody)?;
    Ok(Some(body))
}

/// Test-only convenience: reads a frame from any `AsyncRead` by wrapping it
/// in a `BufReader` first.
#[cfg(test)]
pub async fn read_frame_unbuffered<R: tokio::io::AsyncRead + Unpin>(
    r: R,
) -> Result<Option<Vec<u8>>, FramingError> {
    let mut buffered = tokio::io::BufReader::new(r);
    read_frame(&mut buffered).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn round_trips_a_frame() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, br#"{"hello":"world"}"#)
            .await
            .unwrap();
        let mut reader = BufReader::new(&buf[..]);
        let body = read_frame(&mut reader).await.unwrap().unwrap();
        assert_eq!(body, br#"{"hello":"world"}"#);
    }

    #[tokio::test]
    async fn two_frames_do_not_interleave() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, b"{\"a\":1}").await.unwrap();
        write_frame(&mut buf, b"{\"b\":2}").await.unwrap();
        let mut reader = BufReader::new(&buf[..]);
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap(),
            b"{\"a\":1}"
        );
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap(),
            b"{\"b\":2}"
        );
    }

    #[tokio::test]
    async fn clean_eof_before_any_bytes_is_none() {
        let mut reader = BufReader::new(&b""[..]);
        assert!(read_frame(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn missing_content_length_is_an_error() {
        let mut reader = BufReader::new(&b"X-Other: 1\r\n\r\n{}"[..]);
        assert!(matches!(
            read_frame(&mut reader).await,
            Err(FramingError::MissingContentLength)
        ));
    }

    #[tokio::test]
    async fn duplicate_content_length_is_an_error() {
        let mut reader = BufReader::new(&b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}"[..]);
        assert!(matches!(
            read_frame(&mut reader).await,
            Err(FramingError::MissingContentLength)
        ));
    }

    #[tokio::test]
    async fn body_shorter_than_declared_is_an_error() {
        let mut reader = BufReader::new(&b"Content-Length: 10\r\n\r\n{}"[..]);
        assert!(matches!(
            read_frame(&mut reader).await,
            Err(FramingError::TruncatedBody)
        ));
    }

    #[tokio::test]
    async fn body_over_cap_is_rejected() {
        let header = format!("Content-Length: {}\r\n\r\n", MAX_BODY_BYTES + 1);
        let mut reader = BufReader::new(header.as_bytes());
        assert!(matches!(
            read_frame(&mut reader).await,
            Err(FramingError::BodyTooLarge(_))
        ));
    }

    #[tokio::test]
    async fn split_reads_via_unbuffered_wrapper() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, br#"{"split":true}"#).await.unwrap();
        let body = read_frame_unbuffered(&buf[..]).await.unwrap().unwrap();
        assert_eq!(body, br#"{"split":true}"#);
    }
}
