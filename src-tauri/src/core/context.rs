use serde_json::Value;
use uuid::Uuid;

use crate::core::messages::{ConversationMessage, MessagePart, MessageRole};

const DEFAULT_RECENT_MESSAGE_COUNT: usize = 10;
const DEFAULT_MAX_SUMMARY_CHARS: usize = 6_000;
const SUMMARY_TEXT_LIMIT: usize = 800;

#[derive(Debug, Clone)]
pub struct ContextBuilder {
    recent_message_count: usize,
    max_summary_chars: usize,
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self {
            recent_message_count: DEFAULT_RECENT_MESSAGE_COUNT,
            max_summary_chars: DEFAULT_MAX_SUMMARY_CHARS,
        }
    }
}

impl ContextBuilder {
    pub fn build_model_history(
        &self,
        history: Vec<ConversationMessage>,
    ) -> Vec<ConversationMessage> {
        if history.len() <= self.recent_message_count {
            return history;
        }

        let split_at = self.recent_boundary(&history);
        let older = &history[..split_at];
        let mut compacted = Vec::with_capacity(self.recent_message_count + 1);
        compacted.push(self.summary_message(older));
        compacted.extend_from_slice(&history[split_at..]);
        compacted
    }

    fn recent_boundary(&self, history: &[ConversationMessage]) -> usize {
        let tentative = history.len() - self.recent_message_count;
        let boundary = history[..=tentative]
            .iter()
            .rposition(|message| matches!(message.role, MessageRole::User))
            .unwrap_or(tentative);

        if boundary == 0 {
            self.fallback_boundary(&history[tentative..]) + tentative
        } else {
            boundary
        }
    }

    fn fallback_boundary(&self, recent_candidate: &[ConversationMessage]) -> usize {
        recent_candidate
            .iter()
            .position(|message| !matches!(message.role, MessageRole::Tool))
            .unwrap_or(recent_candidate.len())
    }

    fn summary_message(&self, messages: &[ConversationMessage]) -> ConversationMessage {
        let mut summary = String::from(
            "Earlier conversation summary for context. Raw older tool outputs are stored in session history and are summarized here instead of repeated verbatim.\n",
        );

        for message in messages {
            let line = summarize_message(message);
            if line.is_empty() {
                continue;
            }

            if summary.len() + line.len() + 1 > self.max_summary_chars {
                summary.push_str("- Additional earlier context omitted by context budget.\n");
                break;
            }

            summary.push_str("- ");
            summary.push_str(&line);
            summary.push('\n');
        }

        ConversationMessage {
            id: Uuid::new_v4(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Text { text: summary }],
        }
    }
}

fn summarize_message(message: &ConversationMessage) -> String {
    let mut parts = Vec::new();

    for part in &message.parts {
        match part {
            MessagePart::Text { text } => {
                parts.push(format!(
                    "{} said: {}",
                    role_name(&message.role),
                    truncate_for_summary(text, SUMMARY_TEXT_LIMIT)
                ));
            }
            MessagePart::ToolCall { call } => {
                parts.push(format!(
                    "assistant requested tool `{}` with {}",
                    call.name,
                    summarize_json(&call.arguments)
                ));
            }
            MessagePart::ToolResult { result } => {
                parts.push(format!(
                    "tool `{}` returned {}",
                    result.name,
                    summarize_tool_result(&result.content)
                ));
            }
            MessagePart::Thinking { .. } => {
                parts.push("assistant thinking omitted from compacted context".to_string());
            }
            MessagePart::ApprovalRequest { request } => {
                parts.push(format!(
                    "approval requested for {}: {} ({})",
                    request.requested_capabilities.join(", "),
                    request.summary,
                    request.reason
                ));
            }
            MessagePart::ApprovalDecision { decision } => {
                parts.push(format!(
                    "approval {} by {}: {}",
                    if decision.approved {
                        "approved"
                    } else {
                        "denied"
                    },
                    decision.reviewer,
                    decision.reason
                ));
            }
            MessagePart::Diff { diff } => {
                parts.push(format!("diff proposed: {}", diff.summary));
            }
        }
    }

    parts.join("; ")
}

fn role_name(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

fn summarize_tool_result(content: &Value) -> String {
    let Some(object) = content.as_object() else {
        return summarize_json(content);
    };

    let mut facts = Vec::new();
    for key in [
        "path",
        "query",
        "bytes_read",
        "original_byte_count",
        "truncated",
        "searched_files",
    ] {
        if let Some(value) = object.get(key) {
            facts.push(format!("{key}={}", summarize_json(value)));
        }
    }

    if let Some(entries) = object.get("entries").and_then(Value::as_array) {
        facts.push(format!("entries={}", entries.len()));
    }

    if let Some(hidden_entries) = object.get("hidden_entries").and_then(Value::as_array) {
        facts.push(format!("hidden_entries={}", hidden_entries.len()));
    }

    if let Some(matches) = object.get("matches").and_then(Value::as_array) {
        facts.push(format!("matches={}", matches.len()));
    }

    if facts.is_empty() {
        summarize_json(content)
    } else {
        facts.join(", ")
    }
}

fn summarize_json(value: &Value) -> String {
    match value {
        Value::String(value) => format!("{value:?}"),
        Value::Number(_) | Value::Bool(_) | Value::Null => value.to_string(),
        Value::Array(items) => format!("array(len={})", items.len()),
        Value::Object(object) => {
            let pairs = object
                .iter()
                .filter(|(key, _)| key.as_str() != "content")
                .take(6)
                .map(|(key, value)| format!("{key}={}", summarize_json(value)))
                .collect::<Vec<_>>();
            format!("{{{}}}", pairs.join(", "))
        }
    }
}

fn truncate_for_summary(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let truncated = value.chars().take(max_chars).collect::<String>();
    format!("{truncated}...[truncated]")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::core::messages::{ToolCall, ToolResult};

    use super::*;

    #[test]
    fn context_builder_keeps_short_history_verbatim() {
        let history = vec![text_message(MessageRole::User, "hello")];
        let compacted = ContextBuilder::default().build_model_history(history.clone());

        assert_eq!(compacted.len(), history.len());
        assert_eq!(compacted[0].parts.len(), 1);
    }

    #[test]
    fn context_builder_summarizes_older_tool_results() {
        let mut history = Vec::new();
        history.push(ConversationMessage {
            id: Uuid::new_v4(),
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolResult {
                result: ToolResult {
                    call_id: Some("call_1".to_string()),
                    name: "read_file".to_string(),
                    content: json!({
                        "path": "README.md",
                        "content": "very sensitive or simply very large file contents",
                        "bytes_read": 42,
                        "original_byte_count": 42,
                        "truncated": false,
                    }),
                },
            }],
        });

        for index in 0..12 {
            history.push(text_message(MessageRole::User, &format!("recent {index}")));
        }

        let compacted = ContextBuilder::default().build_model_history(history);
        let summary = match &compacted[0].parts[0] {
            MessagePart::Text { text } => text,
            _ => panic!("expected text summary"),
        };

        assert!(matches!(compacted[0].role, MessageRole::Assistant));
        assert!(summary.contains("tool `read_file` returned"));
        assert!(summary.contains("path=\"README.md\""));
        assert!(summary.contains("bytes_read=42"));
        assert!(!summary.contains("very sensitive"));
        assert_eq!(compacted.len(), DEFAULT_RECENT_MESSAGE_COUNT + 1);
    }

    #[test]
    fn context_builder_summarizes_older_tool_calls() {
        let mut history = vec![ConversationMessage {
            id: Uuid::new_v4(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::ToolCall {
                call: ToolCall {
                    id: Some("call_1".to_string()),
                    name: "search_files".to_string(),
                    arguments: json!({ "path": ".", "query": "OllamaConfig" }),
                },
            }],
        }];

        for index in 0..12 {
            history.push(text_message(
                MessageRole::Assistant,
                &format!("recent {index}"),
            ));
        }

        let compacted = ContextBuilder::default().build_model_history(history);
        let summary = match &compacted[0].parts[0] {
            MessagePart::Text { text } => text,
            _ => panic!("expected text summary"),
        };

        assert!(summary.contains("assistant requested tool `search_files`"));
        assert!(summary.contains("query=\"OllamaConfig\""));
    }

    #[test]
    fn context_builder_keeps_recent_context_on_user_boundary() {
        let mut history = Vec::new();
        history.push(text_message(MessageRole::User, "very old request"));
        history.push(text_message(MessageRole::Assistant, "very old answer"));
        history.push(text_message(MessageRole::User, "older request"));
        history.push(ConversationMessage {
            id: Uuid::new_v4(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::ToolCall {
                call: ToolCall {
                    id: Some("call_1".to_string()),
                    name: "list_files".to_string(),
                    arguments: json!({ "path": "." }),
                },
            }],
        });
        history.push(ConversationMessage {
            id: Uuid::new_v4(),
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolResult {
                result: ToolResult {
                    call_id: Some("call_1".to_string()),
                    name: "list_files".to_string(),
                    content: json!({ "path": ".", "entries": [] }),
                },
            }],
        });

        for index in 0..9 {
            history.push(text_message(
                MessageRole::Assistant,
                &format!("recent {index}"),
            ));
        }

        let compacted = ContextBuilder::default().build_model_history(history);

        assert!(matches!(compacted[1].role, MessageRole::User));
        assert_eq!(compacted.len(), 13);
    }

    #[test]
    fn context_builder_stays_bounded_when_only_user_boundary_is_first_message() {
        let mut history = vec![text_message(MessageRole::User, "initial request")];

        for index in 0..12 {
            history.push(text_message(
                MessageRole::Assistant,
                &format!("assistant detail {index}"),
            ));
        }

        let compacted = ContextBuilder::default().build_model_history(history);

        assert!(compacted.len() <= DEFAULT_RECENT_MESSAGE_COUNT + 1);
        assert!(matches!(compacted[0].role, MessageRole::Assistant));
    }

    #[test]
    fn context_builder_does_not_start_recent_history_with_tool_result() {
        let mut history = vec![
            text_message(MessageRole::User, "initial request"),
            ConversationMessage {
                id: Uuid::new_v4(),
                role: MessageRole::Assistant,
                parts: vec![MessagePart::ToolCall {
                    call: ToolCall {
                        id: Some("call_1".to_string()),
                        name: "read_file".to_string(),
                        arguments: json!({ "path": "README.md" }),
                    },
                }],
            },
            ConversationMessage {
                id: Uuid::new_v4(),
                role: MessageRole::Tool,
                parts: vec![MessagePart::ToolResult {
                    result: ToolResult {
                        call_id: Some("call_1".to_string()),
                        name: "read_file".to_string(),
                        content: json!({ "path": "README.md", "bytes_read": 42 }),
                    },
                }],
            },
        ];

        for index in 0..9 {
            history.push(text_message(
                MessageRole::Assistant,
                &format!("assistant detail {index}"),
            ));
        }

        let compacted = ContextBuilder::default().build_model_history(history);

        assert!(compacted.len() <= DEFAULT_RECENT_MESSAGE_COUNT + 1);
        assert!(matches!(compacted[0].role, MessageRole::Assistant));
        assert!(!matches!(compacted[1].role, MessageRole::Tool));
    }

    fn text_message(role: MessageRole, text: &str) -> ConversationMessage {
        ConversationMessage {
            id: Uuid::new_v4(),
            role,
            parts: vec![MessagePart::Text {
                text: text.to_string(),
            }],
        }
    }
}
