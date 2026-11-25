//! Parsing a streamed Messages API response.
//!
//! [`SseParser`] turns the raw body into server-sent events; [`Accumulator`]
//! folds those events into the finished assistant message (every content
//! block, unchanged, so it can be sent back on the next request), the stop
//! reason and the token usage, and reports the pieces the UI streams.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::client::Usage;

/// One server-sent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Incremental SSE parser: feed body chunks, get complete events.
///
/// Lines are split on bytes, so a multi-byte character cut across two chunks
/// is reassembled before it is decoded.
#[derive(Debug, Default)]
pub struct SseParser {
    pending: Vec<u8>,
    event: String,
    data: Vec<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a chunk; returns the events it completed.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if let Some(event) = self.line(&line) {
                events.push(event);
            }
        }
        events
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            if self.data.is_empty() && self.event.is_empty() {
                return None;
            }
            let event = SseEvent {
                event: std::mem::take(&mut self.event),
                data: std::mem::take(&mut self.data).join("\n"),
            };
            return Some(event);
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = value.to_string(),
            "data" => self.data.push(value.to_string()),
            _ => {}
        }
        None
    }
}

/// Something the UI can show while the answer streams.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    /// Visible answer text.
    Text(String),
    /// A tool call finished streaming its input.
    ToolUse { id: String, name: String },
}

/// Why a stream could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    /// Anthropic sent an `error` event (e.g. `overloaded_error`).
    #[error("the Anthropic API reported an error during the stream")]
    Api { error_type: Option<String> },
    /// An event whose JSON could not be read.
    #[error("unexpected event in the Anthropic API stream")]
    Malformed,
}

#[derive(Debug, Clone)]
struct Block {
    value: Value,
    partial_json: String,
}

/// A tool call from the finished message.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The parsed input; `None` when the streamed JSON did not parse.
    pub input: Option<Value>,
}

/// The finished assistant message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FinishedMessage {
    pub model: Option<String>,
    /// Every content block, in order, as the API produced it.
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
    pub usage: Usage,
    /// The stream ended with `message_stop`.
    pub complete: bool,
}

impl FinishedMessage {
    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.content
            .iter()
            .filter(|block| block["type"] == "tool_use")
            .map(|block| ToolCall {
                id: block["id"].as_str().unwrap_or_default().to_string(),
                name: block["name"].as_str().unwrap_or_default().to_string(),
                input: block
                    .get("input")
                    .filter(|input| input.is_object())
                    .cloned(),
            })
            .collect()
    }

    /// The visible text of the message.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Folds stream events into a [`FinishedMessage`].
#[derive(Debug, Default)]
pub struct Accumulator {
    model: Option<String>,
    blocks: BTreeMap<usize, Block>,
    stop_reason: Option<String>,
    usage: Usage,
    complete: bool,
    /// Tool use blocks whose input JSON did not parse.
    invalid_inputs: Vec<String>,
}

fn merge_usage(into: &mut Usage, from: &Value) {
    if let Some(n) = from["input_tokens"].as_u64() {
        into.input_tokens = n;
    }
    if let Some(n) = from["output_tokens"].as_u64() {
        into.output_tokens = n;
    }
    if let Some(n) = from["cache_read_input_tokens"].as_u64() {
        into.cache_read_input_tokens = Some(n);
    }
    if let Some(n) = from["cache_creation_input_tokens"].as_u64() {
        into.cache_creation_input_tokens = Some(n);
    }
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Usage reported so far (also meaningful after an error).
    pub fn usage(&self) -> &Usage {
        &self.usage
    }

    /// The model that answered, from `message_start`.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Apply one event; returns what the UI may show for it.
    pub fn apply(&mut self, event: &SseEvent) -> Result<Option<Delta>, StreamError> {
        if event.data.is_empty() {
            return Ok(None);
        }
        let data: Value = serde_json::from_str(&event.data).map_err(|_| StreamError::Malformed)?;
        let kind = data["type"].as_str().unwrap_or(event.event.as_str());
        match kind {
            "message_start" => {
                let message = &data["message"];
                self.model = message["model"].as_str().map(str::to_string);
                merge_usage(&mut self.usage, &message["usage"]);
                Ok(None)
            }
            "content_block_start" => {
                let index = block_index(&data)?;
                let mut value = data["content_block"].clone();
                if !value.is_object() {
                    return Err(StreamError::Malformed);
                }
                let initial_text = (value["type"] == "text")
                    .then(|| value["text"].as_str().unwrap_or_default().to_string())
                    .filter(|text| !text.is_empty());
                if value["type"] == "tool_use" {
                    // The input arrives as partial JSON; it is set at block stop.
                    value["input"] = json!({});
                }
                self.blocks.insert(
                    index,
                    Block {
                        value,
                        partial_json: String::new(),
                    },
                );
                Ok(initial_text.map(Delta::Text))
            }
            "content_block_delta" => {
                let index = block_index(&data)?;
                let delta = &data["delta"];
                let Some(block) = self.blocks.get_mut(&index) else {
                    return Err(StreamError::Malformed);
                };
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let text = delta["text"].as_str().unwrap_or_default();
                        append_str(&mut block.value, "text", text);
                        Ok((!text.is_empty()).then(|| Delta::Text(text.to_string())))
                    }
                    "input_json_delta" => {
                        block
                            .partial_json
                            .push_str(delta["partial_json"].as_str().unwrap_or_default());
                        Ok(None)
                    }
                    "thinking_delta" => {
                        append_str(
                            &mut block.value,
                            "thinking",
                            delta["thinking"].as_str().unwrap_or_default(),
                        );
                        Ok(None)
                    }
                    "signature_delta" => {
                        append_str(
                            &mut block.value,
                            "signature",
                            delta["signature"].as_str().unwrap_or_default(),
                        );
                        Ok(None)
                    }
                    // Citations and future delta kinds don't change what is replayed.
                    _ => Ok(None),
                }
            }
            "content_block_stop" => {
                let index = block_index(&data)?;
                let Some(block) = self.blocks.get_mut(&index) else {
                    return Err(StreamError::Malformed);
                };
                if block.value["type"] != "tool_use" {
                    return Ok(None);
                }
                let raw = std::mem::take(&mut block.partial_json);
                let parsed = if raw.trim().is_empty() {
                    Some(json!({}))
                } else {
                    serde_json::from_str::<Value>(&raw)
                        .ok()
                        .filter(Value::is_object)
                };
                let id = block.value["id"].as_str().unwrap_or_default().to_string();
                let name = block.value["name"].as_str().unwrap_or_default().to_string();
                match parsed {
                    Some(input) => block.value["input"] = input,
                    None => self.invalid_inputs.push(id.clone()),
                }
                Ok(Some(Delta::ToolUse { id, name }))
            }
            "message_delta" => {
                if let Some(reason) = data["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_string());
                }
                merge_usage(&mut self.usage, &data["usage"]);
                Ok(None)
            }
            "message_stop" => {
                self.complete = true;
                Ok(None)
            }
            "error" => Err(StreamError::Api {
                error_type: data["error"]["type"]
                    .as_str()
                    .filter(|kind| {
                        kind.len() <= 64
                            && kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                    })
                    .map(str::to_string),
            }),
            // `ping` and unknown events.
            _ => Ok(None),
        }
    }

    /// The message so far. Tool calls whose input did not parse carry no
    /// input (see [`FinishedMessage::tool_calls`]) but keep `{}` in `content`
    /// so the message can still be replayed.
    pub fn finish(self) -> FinishedMessage {
        let invalid = self.invalid_inputs;
        let content = self.blocks.into_values().map(|block| block.value).collect();
        let mut message = FinishedMessage {
            model: self.model,
            content,
            stop_reason: self.stop_reason,
            usage: self.usage,
            complete: self.complete,
        };
        for block in &mut message.content {
            if block["type"] == "tool_use"
                && invalid.iter().any(|id| block["id"].as_str() == Some(id))
            {
                // Marks the call so `tool_calls()` reports no input.
                block["input"] = Value::Null;
            }
        }
        message
    }
}

impl FinishedMessage {
    /// The content to send back to the API: identical, except that a tool
    /// call whose input did not parse carries `{}`.
    pub fn replay_content(&self) -> Vec<Value> {
        self.content
            .iter()
            .map(|block| {
                if block["type"] == "tool_use" && block["input"].is_null() {
                    let mut block = block.clone();
                    block["input"] = json!({});
                    block
                } else {
                    block.clone()
                }
            })
            .collect()
    }
}

fn block_index(data: &Value) -> Result<usize, StreamError> {
    data["index"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(StreamError::Malformed)
}

fn append_str(value: &mut Value, field: &str, text: &str) {
    let current = value[field].as_str().unwrap_or_default();
    value[field] = Value::String(format!("{current}{text}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a recorded stream through parser and accumulator, splitting the
    /// body into chunks of `chunk` bytes to exercise reassembly.
    fn run(body: &str, chunk: usize) -> (Vec<Delta>, Result<FinishedMessage, StreamError>) {
        let mut parser = SseParser::new();
        let mut acc = Accumulator::new();
        let mut deltas = Vec::new();
        for piece in body.as_bytes().chunks(chunk) {
            for event in parser.feed(piece) {
                match acc.apply(&event) {
                    Ok(Some(delta)) => deltas.push(delta),
                    Ok(None) => {}
                    Err(e) => return (deltas, Err(e)),
                }
            }
        }
        (deltas, Ok(acc.finish()))
    }

    const TEXT_STREAM: &str = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-opus-5-5\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":120,\"output_tokens\":1,\"cache_read_input_tokens\":900,\"cache_creation_input_tokens\":0}}}\n\
\n\
event: ping\n\
data: {\"type\": \"ping\"}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig-abc\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Jellyfin is \"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"healthy \u{2014} caf\u{e9}.\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":42}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

    #[test]
    fn text_stream_yields_deltas_full_content_and_usage() {
        for chunk in [1, 7, 64, 4096] {
            let (deltas, message) = run(TEXT_STREAM, chunk);
            let message = message.unwrap();
            assert_eq!(
                deltas,
                vec![
                    Delta::Text("Jellyfin is ".into()),
                    Delta::Text("healthy \u{2014} caf\u{e9}.".into())
                ],
                "chunk size {chunk}"
            );
            assert_eq!(message.text(), "Jellyfin is healthy \u{2014} caf\u{e9}.");
            assert_eq!(message.stop_reason.as_deref(), Some("end_turn"));
            assert!(message.complete);
            assert_eq!(message.model.as_deref(), Some("claude-opus-5-5"));
            assert_eq!(
                message.usage,
                Usage {
                    input_tokens: 120,
                    output_tokens: 42,
                    cache_read_input_tokens: Some(900),
                    cache_creation_input_tokens: Some(0),
                }
            );
            // The thinking block is kept, unchanged, for the next request.
            assert_eq!(
                message.content[0],
                json!({"type": "thinking", "thinking": "", "signature": "sig-abc"})
            );
            assert!(message.tool_calls().is_empty());
        }
    }

    const TOOL_STREAM: &str = "event: message_start\r\n\
data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\r\n\
\r\n\
event: content_block_start\r\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"get_app\",\"input\":{}}}\r\n\
\r\n\
event: content_block_delta\r\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\"}}\r\n\
\r\n\
event: content_block_delta\r\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"na\"}}\r\n\
\r\n\
event: content_block_delta\r\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"me\\\": \\\"jelly\"}}\r\n\
\r\n\
event: content_block_delta\r\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"fin\\\"}\"}}\r\n\
\r\n\
event: content_block_stop\r\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\r\n\
\r\n\
event: content_block_start\r\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"list_apps\",\"input\":{}}}\r\n\
\r\n\
event: content_block_stop\r\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\r\n\
\r\n\
event: message_delta\r\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":30}}\r\n\
\r\n\
event: message_stop\r\n\
data: {\"type\":\"message_stop\"}\r\n\
\r\n";

    #[test]
    fn tool_use_input_is_reassembled_from_split_partial_json() {
        for chunk in [1, 5, 4096] {
            let (deltas, message) = run(TOOL_STREAM, chunk);
            let message = message.unwrap();
            assert_eq!(
                deltas,
                vec![
                    Delta::ToolUse {
                        id: "toolu_1".into(),
                        name: "get_app".into()
                    },
                    Delta::ToolUse {
                        id: "toolu_2".into(),
                        name: "list_apps".into()
                    }
                ]
            );
            assert_eq!(message.stop_reason.as_deref(), Some("tool_use"));
            let calls = message.tool_calls();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0].input, Some(json!({"name": "jellyfin"})));
            // No input deltas at all means an empty object.
            assert_eq!(calls[1].input, Some(json!({})));
            assert_eq!(message.content[0]["input"], json!({"name": "jellyfin"}));
        }
    }

    #[test]
    fn unparseable_tool_input_is_reported_but_still_replayable() {
        let body = "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_x\",\"name\":\"get_app\",\"input\":{}}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"name\\\": \\\"jel\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":16000}}\n\n";
        let (_, message) = run(body, 3);
        let message = message.unwrap();
        assert_eq!(message.stop_reason.as_deref(), Some("max_tokens"));
        assert_eq!(message.tool_calls()[0].input, None);
        assert_eq!(message.replay_content()[0]["input"], json!({}));
        assert!(!message.complete);
    }

    #[test]
    fn refusal_stop_reason_is_reported() {
        let body = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":50,\"output_tokens\":0}}}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\",\"stop_details\":{\"type\":\"refusal\",\"category\":\"cyber\"}},\"usage\":{\"output_tokens\":3}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";
        let (deltas, message) = run(body, 11);
        let message = message.unwrap();
        assert!(deltas.is_empty());
        assert_eq!(message.stop_reason.as_deref(), Some("refusal"));
        assert_eq!(message.usage.output_tokens, 3);
        assert_eq!(message.text(), "");
    }

    #[test]
    fn error_event_stops_with_its_type_only() {
        let body = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":7,\"output_tokens\":1}}}\n\n\
event: error\n\
data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        let (_, result) = run(body, 9);
        assert_eq!(
            result,
            Err(StreamError::Api {
                error_type: Some("overloaded_error".into())
            })
        );
        let body = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"Not An Identifier\"}}\n\n";
        assert_eq!(run(body, 100).1, Err(StreamError::Api { error_type: None }));
    }

    #[test]
    fn max_tokens_keeps_partial_text() {
        let body = "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Half an ans\"}}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":16000}}\n\n\
data: {\"type\":\"message_stop\"}\n\n";
        let (deltas, message) = run(body, 2);
        let message = message.unwrap();
        assert_eq!(deltas, vec![Delta::Text("Half an ans".into())]);
        assert_eq!(message.stop_reason.as_deref(), Some("max_tokens"));
        assert_eq!(message.text(), "Half an ans");
    }

    #[test]
    fn parser_handles_comments_multiline_data_and_missing_space() {
        let mut parser = SseParser::new();
        let events = parser.feed(b": keep-alive\n\nevent:x\ndata:a\ndata: b\n\n");
        assert_eq!(
            events,
            vec![SseEvent {
                event: "x".into(),
                data: "a\nb".into()
            }]
        );
        assert!(parser.feed(b"data: partial").is_empty());
        assert_eq!(parser.feed(b"\n\n")[0].data, "partial");
    }

    #[test]
    fn malformed_json_and_unknown_index_are_errors() {
        let mut acc = Accumulator::new();
        let bad = SseEvent {
            event: "content_block_delta".into(),
            data: "{not json".into(),
        };
        assert_eq!(acc.apply(&bad), Err(StreamError::Malformed));
        let orphan = SseEvent {
            event: "content_block_delta".into(),
            data: r#"{"type":"content_block_delta","index":3,"delta":{"type":"text_delta","text":"x"}}"#
                .into(),
        };
        assert_eq!(acc.apply(&orphan), Err(StreamError::Malformed));
    }
}
