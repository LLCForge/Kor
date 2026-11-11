//! Byte stream → frames (SSE or NDJSON) → `StreamEvent`s, with cancellation.

use futures::{SinkExt, StreamExt};
use kor_core::{FinishReason, ProviderError, ProviderErrorKind, StreamEvent};
use reqwest::Response;
use tokio_util::sync::CancellationToken;

use crate::EventStream;
use crate::http::Http;

/// A single line longer than this aborts the stream (protects memory).
const MAX_LINE: usize = 8 * 1024 * 1024;

/// One logical message from the wire. For NDJSON `event` is always `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Framing {
    Sse,
    Ndjson,
}

/// Provider-specific translation of frames into events.
pub(crate) trait Mapper: Send + 'static {
    fn frame(&mut self, frame: Frame) -> Result<Vec<StreamEvent>, ProviderError>;
    /// Called once the body ends; must emit `Finish` or fail.
    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError>;
}

/// Splits bytes into lines, tolerating chunk boundaries anywhere (even inside UTF-8).
#[derive(Debug, Default)]
pub(crate) struct Lines {
    buf: Vec<u8>,
}

impl Lines {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            out.push(String::from_utf8_lossy(&line).into_owned());
        }
        if self.buf.len() > MAX_LINE {
            return Err("stream line exceeds limit".into());
        }
        Ok(out)
    }

    pub(crate) fn rest(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        let s = String::from_utf8_lossy(&rest)
            .trim_end_matches('\r')
            .to_owned();
        (!s.is_empty()).then_some(s)
    }
}

/// Server-sent events state machine (WHATWG subset: `event`, `data`, comments).
#[derive(Debug, Default)]
pub(crate) struct Sse {
    event: Option<String>,
    data: Vec<String>,
}

impl Sse {
    pub(crate) fn line(&mut self, line: &str) -> Option<Frame> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
        None
    }

    pub(crate) fn dispatch(&mut self) -> Option<Frame> {
        let event = self.event.take();
        if self.data.is_empty() {
            return None;
        }
        Some(Frame {
            event,
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }
}

impl Framing {
    fn feed(self, sse: &mut Sse, line: &str) -> Option<Frame> {
        match self {
            Self::Sse => sse.line(line),
            Self::Ndjson => (!line.trim().is_empty()).then(|| Frame {
                event: None,
                data: line.to_owned(),
            }),
        }
    }

    fn end(self, sse: &mut Sse, rest: Option<String>) -> Vec<Frame> {
        let mut out: Vec<Frame> = rest.and_then(|l| self.feed(sse, &l)).into_iter().collect();
        if let Self::Sse = self {
            out.extend(sse.dispatch());
        }
        out
    }
}

/// Drive `resp` on a background task and expose the mapped events as a stream.
/// Dropping the stream or cancelling `cancel` stops the task and closes the connection.
pub(crate) fn spawn(
    http: &Http,
    resp: Response,
    framing: Framing,
    mut mapper: impl Mapper,
    cancel: CancellationToken,
) -> EventStream {
    let (mut tx, rx) = futures::channel::mpsc::channel(64);
    let http = http.clone();
    tokio::spawn(async move {
        let mut body = resp.bytes_stream();
        let mut lines = Lines::default();
        let mut sse = Sse::default();
        let result: Result<(), ProviderError> = async {
            loop {
                let next = tokio::select! {
                    biased;
                    () = cancel.cancelled() => {
                        let _ = tx.send(Ok(StreamEvent::Finish { reason: FinishReason::Cancelled })).await;
                        return Ok(());
                    }
                    n = body.next() => n,
                };
                let ended = next.is_none();
                let frames = match next {
                    None => framing.end(&mut sse, lines.rest()),
                    Some(Err(e)) => return Err(http.transport(&e)),
                    Some(Ok(bytes)) => lines
                        .push(&bytes)
                        .map_err(|m| http.err(ProviderErrorKind::Protocol, m))?
                        .iter()
                        .filter_map(|l| framing.feed(&mut sse, l))
                        .collect(),
                };
                for f in frames {
                    for ev in mapper.frame(f)? {
                        if tx.send(Ok(ev)).await.is_err() {
                            return Ok(());
                        }
                    }
                }
                if ended {
                    for ev in mapper.finish()? {
                        if tx.send(Ok(ev)).await.is_err() {
                            return Ok(());
                        }
                    }
                    return Ok(());
                }
            }
        }
        .await;
        if let Err(e) = result {
            let _ = tx.send(Err(e)).await;
        }
    });
    Box::pin(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_survive_arbitrary_chunking() {
        let input = "data: {\"a\":\"ğü\"}\r\n\r\nevent: x\ndata: 1\n\n";
        for split in 1..input.len() {
            let bytes = input.as_bytes();
            let mut l = Lines::default();
            let mut sse = Sse::default();
            let mut frames = Vec::new();
            for chunk in [&bytes[..split], &bytes[split..]] {
                for line in l.push(chunk).unwrap() {
                    frames.extend(sse.line(&line));
                }
            }
            assert_eq!(frames.len(), 2, "split {split}");
            assert_eq!(frames[0].data, "{\"a\":\"ğü\"}");
            assert_eq!(frames[1].event.as_deref(), Some("x"));
        }
    }

    #[test]
    fn sse_multiline_data_and_comments() {
        let mut sse = Sse::default();
        assert!(sse.line(": keepalive").is_none());
        assert!(sse.line("data: a").is_none());
        assert!(sse.line("data:b").is_none());
        let f = sse.line("").unwrap();
        assert_eq!(f.data, "a\nb");
        assert!(sse.line("").is_none());
    }

    #[test]
    fn trailing_frame_without_blank_line_is_flushed() {
        let mut sse = Sse::default();
        let frames = Framing::Sse.end(&mut sse, Some("data: last".into()));
        assert_eq!(frames.len(), 1);
        let frames = Framing::Ndjson.end(&mut sse, Some("{\"done\":true}".into()));
        assert_eq!(frames[0].data, "{\"done\":true}");
    }

    #[test]
    fn oversized_line_is_rejected() {
        let mut l = Lines::default();
        assert!(l.push(&vec![b'a'; MAX_LINE + 1]).is_err());
    }
}
