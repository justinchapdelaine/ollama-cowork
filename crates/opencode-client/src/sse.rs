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
    let mut data = String::new();
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
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
}
