//! Line-delimited JSON reader on the `claude` subprocess's stdout.
//!
//! A real capture (this wave, against `claude` 2.1.239) showed stdout is
//! *not* purely one-JSON-object-per-line: an invalid-model run emitted a
//! bracketed diagnostic line (`[claude-code:unrecognized_model] {...}`)
//! ahead of the JSON frames. `RawLine::Malformed` exists precisely so that
//! kind of line degrades gracefully (LLD-07 §8-4) instead of desyncing the
//! parser — framing recovers line-by-line since we never cross a line
//! boundary.

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdout;

pub enum RawLine {
    Frame(Value),
    /// A line that isn't valid JSON. Counted toward the 3-consecutive cap
    /// (LLD-07 §8-4) by the caller.
    Malformed,
}

pub struct FrameReader {
    lines: tokio::io::Lines<BufReader<ChildStdout>>,
}

impl FrameReader {
    pub fn new(stdout: ChildStdout) -> Self {
        Self {
            lines: BufReader::new(stdout).lines(),
        }
    }

    /// `Ok(None)` is a clean EOF (the child closed stdout).
    pub async fn next_raw(&mut self) -> std::io::Result<Option<RawLine>> {
        loop {
            let Some(line) = self.lines.next_line().await? else {
                return Ok(None);
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue; // blank lines are not malformed — just noise.
            }
            return Ok(Some(match serde_json::from_str::<Value>(trimmed) {
                Ok(v) => RawLine::Frame(v),
                Err(_) => RawLine::Malformed,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    async fn reader_over(script: &str) -> (tokio::process::Child, FrameReader) {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn sh");
        let stdout = child.stdout.take().expect("piped stdout");
        (child, FrameReader::new(stdout))
    }

    #[tokio::test]
    async fn parses_a_well_formed_frame() {
        let (mut child, mut fr) = reader_over(r#"printf '%s\n' '{"type":"system"}'"#).await;
        match fr.next_raw().await.unwrap() {
            Some(RawLine::Frame(v)) => assert_eq!(v["type"], "system"),
            _ => panic!("expected a frame"),
        }
        let _ = child.wait().await;
    }

    #[tokio::test]
    async fn skips_blank_lines_without_treating_them_as_malformed() {
        let (mut child, mut fr) = reader_over(r#"printf '\n\n%s\n' '{"type":"ok"}'"#).await;
        match fr.next_raw().await.unwrap() {
            Some(RawLine::Frame(v)) => assert_eq!(v["type"], "ok"),
            _ => panic!("expected the frame after blank lines, not a Malformed marker"),
        }
        let _ = child.wait().await;
    }

    #[tokio::test]
    async fn non_json_line_is_malformed_not_a_panic() {
        let (mut child, mut fr) =
            reader_over(r#"printf '%s\n' '[claude-code:unrecognized_model] {"model":"x"}'"#).await;
        match fr.next_raw().await.unwrap() {
            Some(RawLine::Malformed) => {}
            _ => panic!("expected Malformed for a non-JSON line"),
        }
        let _ = child.wait().await;
    }

    #[tokio::test]
    async fn eof_yields_none() {
        let (mut child, mut fr) = reader_over("true").await;
        assert!(fr.next_raw().await.unwrap().is_none());
        let _ = child.wait().await;
    }
}
