use serde::Deserialize;
use serde_json::Value;
use std::io::BufRead;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OpencodeEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub properties: Value,
}

#[derive(Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<OpencodeEvent>, String> {
        const MAX_PENDING_FRAME_BYTES: usize = 1024 * 1024;
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((end, delimiter_len)) = frame_end(&self.buffer) {
            let frame = self.buffer.drain(..end).collect::<Vec<_>>();
            self.buffer.drain(..delimiter_len);
            let frame = std::str::from_utf8(&frame).map_err(|error| error.to_string())?;
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() {
                emit(&data, &mut |event| {
                    events.push(event);
                    true
                })?;
            }
        }
        if self.buffer.len() > MAX_PENDING_FRAME_BYTES {
            return Err("SSE frame exceeds the configured limit".into());
        }
        Ok(events)
    }
}

fn frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4));
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2));
    match (crlf, lf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn emit(data: &str, callback: &mut impl FnMut(OpencodeEvent) -> bool) -> Result<bool, String> {
    let value: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
    let event_value = value.get("payload").unwrap_or(&value).clone();
    let event: OpencodeEvent = serde_json::from_value(event_value).map_err(|e| e.to_string())?;
    Ok(callback(event))
}

pub fn read_sse(
    reader: impl BufRead,
    callback: &mut impl FnMut(OpencodeEvent) -> bool,
) -> Result<(), String> {
    read_sse_cancellable(reader, callback, || false)
}

pub fn read_sse_cancellable(
    mut reader: impl BufRead,
    callback: &mut impl FnMut(OpencodeEvent) -> bool,
    mut cancelled: impl FnMut() -> bool,
) -> Result<(), String> {
    let mut data = String::new();
    loop {
        if cancelled() {
            return Ok(());
        }
        let mut line = String::new();
        let read = match reader.read_line(&mut line) {
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        if read == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if !data.is_empty() {
                let keep_going = emit(&data, callback)?;
                data.clear();
                if !keep_going {
                    return Ok(());
                }
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n')
            }
            data.push_str(value.trim_start())
        }
    }
    if !data.is_empty() {
        let _ = emit(&data, callback)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_frames_and_stops() {
        let input = b"data: {\"type\":\"session.idle\",\"properties\":{\"sessionID\":\"one\"}}\n\n";
        let mut events = Vec::new();
        read_sse(&input[..], &mut |event| {
            events.push(event);
            false
        })
        .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "session.idle");
    }
    #[test]
    fn unwraps_live_payload_shape() {
        let input = b"data: {\"payload\":{\"type\":\"permission.asked\",\"properties\":{}}}\n\n";
        let mut event_type = String::new();
        read_sse(&input[..], &mut |event| {
            event_type = event.event_type;
            false
        })
        .unwrap();
        assert_eq!(event_type, "permission.asked");
    }

    #[test]
    fn incremental_decoder_preserves_split_utf8_and_crlf_frames() {
        let mut decoder = SseDecoder::default();
        let bytes =
            "data: {\"type\":\"message\",\"properties\":{\"text\":\"café\"}}\r\n\r\n".as_bytes();
        let split = bytes.iter().position(|byte| *byte == 0xc3).unwrap() + 1;
        assert!(decoder.push(&bytes[..split]).unwrap().is_empty());
        let events = decoder.push(&bytes[split..]).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "message");
    }

    #[test]
    fn incremental_decoder_bounds_unterminated_frames() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(&vec![b'x'; 1024 * 1024 + 1]).is_err());
    }
}
