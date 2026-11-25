//! One chat turn: the streaming Messages API tool loop.
//!
//! The question goes to the model with the recent history (last
//! [`MAX_HISTORY_TURNS`] turns plus a running summary of older ones) and a
//! context message. While the model calls tools, Kubarr runs them with the
//! user's permissions and sends the results back, until the model ends its
//! turn or a limit is reached. The UI receives server-sent events:
//! `status`, `text_delta`, `sources`, `done` and `error`.
//!
//! The budget is checked before, and usage recorded after, every API call.
//! The partial answer is saved as it streams; if the browser goes away the
//! answer is saved as `interrupted`.

use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::{json, Value};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::client::{ClientError, Message, MessagesRequest, OutputConfig};
use super::conversations::{self, ChatContext, MessageContent};
use super::pricing::AssistantModel;
use super::prompt::{context_message, SUMMARY_PROMPT, SYSTEM_PROMPT};
use super::stream::{Accumulator, Delta, SseParser, StreamError};
use super::tools::{self, Source, ToolContext};
use super::{budget, record_usage, AssistantSettings, UsageRecord};
use crate::models::assistant_conversation;
use crate::models::assistant_message::{
    self, STATUS_COMPLETE, STATUS_ERROR, STATUS_INTERRUPTED, STATUS_STREAMING,
};

/// Tool calls allowed while answering one question.
pub const MAX_TOOL_CALLS: usize = 15;
/// API calls allowed while answering one question (tool rounds and pauses).
const MAX_ROUNDS: usize = 24;
/// Turns sent in full; older ones are folded into the running summary.
pub const MAX_HISTORY_TURNS: usize = 20;
const MAX_TOKENS: u32 = 16_000;
const SUMMARY_MAX_TOKENS: u32 = 1_024;
/// Save the partial answer at most this often while text streams.
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);

pub const REFUSAL_TEXT: &str = "I can't help with that request.";
const CUT_OFF_TEXT: &str = "_(The answer was cut off because it got too long.)_";
const TOOL_LIMIT_TEXT: &str = "Tool call limit reached for this answer. Do not call more \
tools; answer with what you have found so far.";

/// An event for the browser.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    Status(String),
    TextDelta(String),
    Sources(Vec<Source>),
    Done(i64),
    Error(String),
}

impl ChatEvent {
    pub fn name(&self) -> &'static str {
        match self {
            ChatEvent::Status(_) => "status",
            ChatEvent::TextDelta(_) => "text_delta",
            ChatEvent::Sources(_) => "sources",
            ChatEvent::Done(_) => "done",
            ChatEvent::Error(_) => "error",
        }
    }

    pub fn data(&self) -> Value {
        match self {
            ChatEvent::Status(text) | ChatEvent::TextDelta(text) => json!({ "text": text }),
            ChatEvent::Sources(items) => json!({ "items": items }),
            ChatEvent::Done(id) => json!({ "message_id": id }),
            ChatEvent::Error(message) => json!({ "message": message }),
        }
    }

    /// The event in SSE wire format.
    pub fn to_sse(&self) -> String {
        format!("event: {}\ndata: {}\n\n", self.name(), self.data())
    }
}

/// Writes events to the response body; notices when the browser is gone.
pub struct EventSink<W> {
    writer: W,
    open: bool,
}

impl<W: AsyncWrite + Unpin> EventSink<W> {
    pub fn new(writer: W) -> Self {
        Self { writer, open: true }
    }

    /// Send an event; false once the browser has disconnected.
    pub async fn send(&mut self, event: ChatEvent) -> bool {
        if !self.open {
            return false;
        }
        let bytes = event.to_sse();
        let written = async {
            self.writer.write_all(bytes.as_bytes()).await?;
            self.writer.flush().await
        }
        .await;
        if written.is_err() {
            self.open = false;
        }
        self.open
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

/// Everything one turn needs.
pub struct Turn {
    pub ctx: ToolContext,
    pub conversation: assistant_conversation::Model,
    pub question: String,
    pub question_id: i64,
    pub answer_id: i64,
    pub context: ChatContext,
    pub settings: AssistantSettings,
    pub api_key: String,
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEnd {
    Complete,
    Interrupted,
    Error(String),
}

/// The answer as it builds up.
struct Answer {
    text: String,
    sources: Vec<Source>,
    /// A new round's first text gets a paragraph break.
    round_text_started: bool,
}

impl Answer {
    fn push_source(&mut self, source: Source) {
        if !self
            .sources
            .iter()
            .any(|s| s.tool == source.tool && s.target == source.target)
        {
            self.sources.push(source);
        }
    }

    fn content(&self, error: Option<String>) -> MessageContent {
        MessageContent {
            text: self.text.clone(),
            sources: self.sources.clone(),
            error,
        }
    }
}

fn supports_fallbacks(model: AssistantModel) -> bool {
    matches!(model, AssistantModel::Opus55 | AssistantModel::Sonnet55)
}

/// A sentence for the UI: fixed text per failure class.
pub fn client_error_text(e: &ClientError) -> String {
    match e {
        ClientError::Status { status, error_type } => match (error_type.as_deref(), *status) {
            (Some("authentication_error"), _) | (_, 401) => {
                "Anthropic rejected the API key. An administrator can check it in Settings → Assistant."
            }
            (Some("permission_error"), _) | (_, 403) | (Some("not_found_error"), _) | (_, 404) => {
                "The API key cannot use the selected model."
            }
            (Some("billing_error"), _) | (_, 402) => {
                "The Anthropic account has a billing problem (check credits)."
            }
            (Some("rate_limit_error"), _) | (_, 429) => {
                "Anthropic is rate-limiting requests. Try again shortly."
            }
            (Some("overloaded_error"), _) | (_, 529) | (_, 500..=599) => {
                "Anthropic is temporarily unavailable. Try again in a moment."
            }
            _ => "Anthropic could not process this request.",
        }
        .to_string(),
        ClientError::Transport => "Could not reach the Anthropic API.".to_string(),
        ClientError::Decode => "Anthropic sent an unexpected response.".to_string(),
    }
}

fn stream_error_text(e: &StreamError) -> String {
    match e {
        StreamError::Api { error_type } => client_error_text(&ClientError::Status {
            status: 0,
            error_type: error_type.clone(),
        }),
        StreamError::Malformed => "Anthropic sent an unexpected response.".to_string(),
    }
}

fn system_blocks(text: &str) -> Value {
    json!([{ "type": "text", "text": text, "cache_control": { "type": "ephemeral" } }])
}

async fn record(turn: &Turn, model: Option<&str>, usage: super::client::Usage) {
    let model = model
        .and_then(AssistantModel::parse)
        .unwrap_or(turn.settings.model);
    let result = record_usage(
        &turn.ctx.db,
        &turn.ctx.state.notification,
        UsageRecord {
            user_id: Some(turn.ctx.user_id),
            conversation_id: Some(turn.conversation.id.clone()),
            model,
            usage,
        },
    )
    .await;
    if let Err(e) = result {
        tracing::warn!("Failed to record assistant usage: {e}");
    }
}

/// The text of a stored message.
fn stored_text(message: &assistant_message::Model) -> String {
    MessageContent::from_json(&message.content_json).text
}

/// A question and the answers that followed it.
struct HistoryTurn {
    question: assistant_message::Model,
    answers: Vec<assistant_message::Model>,
}

impl HistoryTurn {
    fn last_id(&self) -> i64 {
        self.answers.last().map_or(self.question.id, |a| a.id)
    }

    fn answer_text(&self) -> String {
        self.answers
            .iter()
            .map(stored_text)
            .filter(|t| !t.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

fn group_turns(messages: Vec<assistant_message::Model>) -> Vec<HistoryTurn> {
    let mut turns: Vec<HistoryTurn> = Vec::new();
    for message in messages {
        if message.role == "user" {
            turns.push(HistoryTurn {
                question: message,
                answers: Vec::new(),
            });
        } else if let Some(turn) = turns.last_mut() {
            turn.answers.push(message);
        }
    }
    turns
}

/// Previous turns as API messages: questions and answer texts only (no tool
/// traffic or thinking from earlier turns), led by the running summary.
async fn history<W: AsyncWrite + Unpin>(
    turn: &Turn,
    sink: &mut EventSink<W>,
) -> crate::error::Result<Vec<Message>> {
    let earlier: Vec<assistant_message::Model> =
        conversations::messages(&turn.ctx.db, &turn.conversation.id)
            .await?
            .into_iter()
            .filter(|m| m.id < turn.question_id)
            .collect();
    let mut turns = group_turns(earlier);
    let mut summary = turn.conversation.summary.clone();
    if turns.len() > MAX_HISTORY_TURNS {
        let recent = turns.split_off(turns.len() - MAX_HISTORY_TURNS);
        let older = turns;
        turns = recent;
        let covered = turn.conversation.summary_through.unwrap_or(0);
        let new: Vec<&HistoryTurn> = older.iter().filter(|t| t.last_id() > covered).collect();
        if let Some(last) = new.last() {
            let through = last.last_id();
            sink.send(ChatEvent::Status(
                "Catching up on the earlier conversation…".into(),
            ))
            .await;
            if let Some(updated) = summarize(turn, summary.as_deref(), &new).await {
                if let Err(e) = conversations::save_summary(
                    &turn.ctx.db,
                    &turn.conversation.id,
                    &updated,
                    through,
                )
                .await
                {
                    tracing::warn!("Failed to save the conversation summary: {e}");
                }
                summary = Some(updated);
            }
        }
    }
    let mut messages = Vec::new();
    if let Some(summary) = summary.filter(|s| !s.trim().is_empty()) {
        messages.push(Message::user(format!(
            "(Summary of the earlier part of this conversation)\n{summary}"
        )));
    }
    for past in &turns {
        messages.push(Message::user(stored_text(&past.question)));
        let answer = past.answer_text();
        if !answer.is_empty() {
            messages.push(Message {
                role: "assistant".into(),
                content: Value::String(answer),
            });
        }
    }
    Ok(messages)
}

/// Fold `turns` into the summary with one small, non-streaming call.
/// `None` when it could not be made (the old summary is kept).
async fn summarize(turn: &Turn, previous: Option<&str>, turns: &[&HistoryTurn]) -> Option<String> {
    if budget::ensure_budget(&turn.ctx.db).await.is_err() {
        return None;
    }
    let mut transcript = String::new();
    if let Some(previous) = previous {
        transcript.push_str(&format!("Previous summary:\n{previous}\n\n"));
    }
    transcript.push_str("Next part of the conversation:\n");
    for t in turns {
        transcript.push_str(&format!("User: {}\n", stored_text(&t.question)));
        transcript.push_str(&format!("Assistant: {}\n", t.answer_text()));
    }
    let request = MessagesRequest {
        model: turn.settings.model.id().to_string(),
        max_tokens: SUMMARY_MAX_TOKENS,
        messages: vec![Message::user(transcript)],
        system: Some(Value::String(SUMMARY_PROMPT.into())),
        output_config: Some(OutputConfig {
            effort: Some("low".into()),
        }),
        ..Default::default()
    };
    match turn
        .ctx
        .state
        .assistant
        .client
        .create(&turn.api_key, &request)
        .await
    {
        Ok(response) => {
            record(turn, Some(&response.model), response.usage.clone()).await;
            if response.stop_reason.as_deref() == Some("refusal") {
                return None;
            }
            let text: String = response
                .content
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect();
            (!text.trim().is_empty()).then_some(text)
        }
        Err(e) => {
            tracing::info!(error = %e, "Assistant summary request failed");
            None
        }
    }
}

/// Run the turn, streaming events to `sink`, and save the answer.
pub async fn run_turn<W: AsyncWrite + Unpin>(turn: Turn, sink: &mut EventSink<W>) -> TurnEnd {
    let mut answer = Answer {
        text: String::new(),
        sources: Vec::new(),
        round_text_started: false,
    };
    let end = run_loop(&turn, sink, &mut answer).await;
    let (status, error) = match &end {
        TurnEnd::Complete => (STATUS_COMPLETE, None),
        TurnEnd::Interrupted => (STATUS_INTERRUPTED, None),
        TurnEnd::Error(message) => (STATUS_ERROR, Some(message.clone())),
    };
    if !answer.sources.is_empty() {
        sink.send(ChatEvent::Sources(answer.sources.clone())).await;
    }
    if let Err(e) =
        conversations::save_answer(&turn.ctx.db, turn.answer_id, &answer.content(error), status)
            .await
    {
        tracing::warn!("Failed to save the assistant answer: {e}");
    }
    match &end {
        TurnEnd::Complete => {
            sink.send(ChatEvent::Done(turn.answer_id)).await;
        }
        TurnEnd::Error(message) => {
            sink.send(ChatEvent::Error(message.clone())).await;
        }
        TurnEnd::Interrupted => {}
    }
    end
}

async fn flush(turn: &Turn, answer: &Answer) {
    if let Err(e) = conversations::save_answer(
        &turn.ctx.db,
        turn.answer_id,
        &answer.content(None),
        STATUS_STREAMING,
    )
    .await
    {
        tracing::debug!("Failed to save the partial assistant answer: {e}");
    }
}

/// Append answer text and stream it; false when the browser is gone.
async fn emit_text<W: AsyncWrite + Unpin>(
    answer: &mut Answer,
    sink: &mut EventSink<W>,
    text: &str,
) -> bool {
    let mut piece = String::new();
    if !answer.round_text_started {
        answer.round_text_started = true;
        if !answer.text.is_empty() {
            piece.push_str("\n\n");
        }
    }
    piece.push_str(text);
    answer.text.push_str(&piece);
    sink.send(ChatEvent::TextDelta(piece)).await
}

async fn run_loop<W: AsyncWrite + Unpin>(
    turn: &Turn,
    sink: &mut EventSink<W>,
    answer: &mut Answer,
) -> TurnEnd {
    let tools = turn.ctx.tools();
    let definitions: Vec<Value> = tools.iter().map(|t| t.definition()).collect();
    let mut messages = match history(turn, sink).await {
        Ok(messages) => messages,
        Err(e) => {
            tracing::warn!("Failed to load the conversation history: {e}");
            return TurnEnd::Error("Could not load the conversation.".into());
        }
    };
    messages.push(Message::user(turn.question.clone()));
    messages.push(Message {
        role: "system".into(),
        content: Value::String(context_message(
            &turn.ctx,
            &turn.context,
            &tools,
            Utc::now(),
        )),
    });

    let mut tool_calls = 0usize;
    let mut tools_closed = false;
    for _round in 0..MAX_ROUNDS {
        if !sink.is_open() {
            return TurnEnd::Interrupted;
        }
        if let Err(e) = budget::ensure_budget(&turn.ctx.db).await {
            return TurnEnd::Error(match e {
                crate::error::AppError::ServiceUnavailable(message) => message,
                _ => "Could not check the assistant budget.".into(),
            });
        }
        flush(turn, answer).await;
        answer.round_text_started = false;
        let request = MessagesRequest {
            model: turn.settings.model.id().to_string(),
            max_tokens: MAX_TOKENS,
            messages: messages.clone(),
            system: Some(system_blocks(SYSTEM_PROMPT)),
            output_config: Some(OutputConfig {
                effort: Some(turn.settings.effort.as_str().to_string()),
            }),
            tools: (!definitions.is_empty()).then(|| definitions.clone()),
            tool_choice: (!definitions.is_empty()).then(|| {
                if tools_closed {
                    json!({"type": "none"})
                } else {
                    json!({"type": "auto"})
                }
            }),
            stream: Some(true),
            fallbacks: supports_fallbacks(turn.settings.model).then(|| json!("default")),
        };
        let mut body = match turn
            .ctx
            .state
            .assistant
            .client
            .stream(&turn.api_key, &request)
            .await
        {
            Ok(body) => body,
            Err(e) => {
                tracing::info!(error = %e, "Assistant request failed");
                return TurnEnd::Error(client_error_text(&e));
            }
        };

        let mut parser = SseParser::new();
        let mut acc = Accumulator::new();
        let mut failure: Option<String> = None;
        let mut gone = false;
        let mut last_flush = Instant::now();
        'read: loop {
            let chunk = match body.next_chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(e) => {
                    failure = Some(client_error_text(&e));
                    break;
                }
            };
            for event in parser.feed(&chunk) {
                match acc.apply(&event) {
                    Ok(Some(Delta::Text(text))) => {
                        if !emit_text(answer, sink, &text).await {
                            gone = true;
                            break 'read;
                        }
                        if last_flush.elapsed() >= FLUSH_INTERVAL {
                            flush(turn, answer).await;
                            last_flush = Instant::now();
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        failure = Some(stream_error_text(&e));
                        break 'read;
                    }
                }
            }
        }
        drop(body);
        let model = acc.model().map(str::to_string);
        let message = acc.finish();
        record(turn, model.as_deref(), message.usage.clone()).await;
        if gone {
            return TurnEnd::Interrupted;
        }
        if let Some(failure) = failure {
            return TurnEnd::Error(failure);
        }

        match message.stop_reason.as_deref() {
            Some("refusal") => {
                let text = if answer.text.is_empty() {
                    REFUSAL_TEXT.to_string()
                } else {
                    format!("\n\n{REFUSAL_TEXT}")
                };
                answer.text.push_str(&text);
                if !sink.send(ChatEvent::TextDelta(text)).await {
                    return TurnEnd::Interrupted;
                }
                return TurnEnd::Complete;
            }
            Some("max_tokens") => {
                let text = format!("\n\n{CUT_OFF_TEXT}");
                answer.text.push_str(&text);
                sink.send(ChatEvent::TextDelta(text)).await;
                return TurnEnd::Complete;
            }
            Some("pause_turn") => {
                messages.push(Message {
                    role: "assistant".into(),
                    content: Value::Array(message.replay_content()),
                });
                continue;
            }
            Some("tool_use") => {
                let calls = message.tool_calls();
                messages.push(Message {
                    role: "assistant".into(),
                    content: Value::Array(message.replay_content()),
                });
                let mut results = Vec::with_capacity(calls.len());
                for call in calls {
                    if tool_calls >= MAX_TOOL_CALLS {
                        tools_closed = true;
                        results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": call.id,
                            "content": TOOL_LIMIT_TEXT,
                            "is_error": true,
                        }));
                        continue;
                    }
                    tool_calls += 1;
                    if !sink
                        .send(ChatEvent::Status(tools::status_line(
                            &call.name,
                            call.input.as_ref(),
                        )))
                        .await
                    {
                        return TurnEnd::Interrupted;
                    }
                    let outcome = tools::execute(&turn.ctx, &call.name, call.input.as_ref()).await;
                    answer.push_source(outcome.source.clone());
                    results.push(outcome.result_block(&call.id));
                }
                if tool_calls >= MAX_TOOL_CALLS {
                    tools_closed = true;
                }
                messages.push(Message {
                    role: "user".into(),
                    content: Value::Array(results),
                });
                continue;
            }
            Some(_) => return TurnEnd::Complete,
            None => {
                return TurnEnd::Error("The answer stopped unexpectedly. Ask again.".into());
            }
        }
    }
    TurnEnd::Error("The assistant took too many steps to answer. Try a narrower question.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_have_the_documented_wire_shape() {
        assert_eq!(
            ChatEvent::TextDelta("hi \"x\"".into()).to_sse(),
            "event: text_delta\ndata: {\"text\":\"hi \\\"x\\\"\"}\n\n"
        );
        assert_eq!(
            ChatEvent::Done(7).to_sse(),
            "event: done\ndata: {\"message_id\":7}\n\n"
        );
        assert_eq!(ChatEvent::Status("s".into()).name(), "status");
        assert_eq!(ChatEvent::Error("e".into()).data(), json!({"message": "e"}));
        assert_eq!(ChatEvent::Sources(vec![]).data(), json!({"items": []}));
    }

    #[tokio::test]
    async fn sink_notices_a_closed_reader() {
        let (writer, reader) = tokio::io::duplex(64);
        let mut sink = EventSink::new(writer);
        drop(reader);
        assert!(!sink.send(ChatEvent::Status("x".into())).await);
        assert!(!sink.is_open());
    }

    #[test]
    fn fallbacks_only_where_supported() {
        assert!(supports_fallbacks(AssistantModel::Opus55));
        assert!(supports_fallbacks(AssistantModel::Sonnet55));
        assert!(!supports_fallbacks(AssistantModel::Haiku55));
    }

    #[test]
    fn errors_never_carry_provider_text() {
        let text = client_error_text(&ClientError::Status {
            status: 400,
            error_type: Some("invalid_request_error".into()),
        });
        assert_eq!(text, "Anthropic could not process this request.");
    }
}
