use crate::core::context::ContextBuilder;
use crate::core::error::{AppError, AppResult};
use crate::core::messages::{ConversationMessage, MessagePart, MessageRole, ToolResult};
use crate::core::model::{ChatRequest, ChatResponse, ModelBackend, ThinkMode, ToolDefinition};
use crate::core::run::CancellationFlag;
use crate::core::tools::{ToolExecutionRequest, ToolRegistry};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_TOOL_ITERATIONS: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTurnRequest {
    pub model: String,
    pub history: Vec<ConversationMessage>,
    pub user_prompt: String,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTurnResponse {
    pub messages: Vec<ConversationMessage>,
    pub done_reason: Option<String>,
    pub tool_iteration_count: usize,
}

pub async fn run_agent_turn(
    backend: &dyn ModelBackend,
    tool_registry: &dyn ToolRegistry,
    request: AgentTurnRequest,
    cancellation: CancellationFlag,
) -> AppResult<AgentTurnResponse> {
    if request.user_prompt.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "user prompt cannot be empty".to_string(),
        ));
    }

    let system = ConversationMessage {
        id: Uuid::new_v4(),
        role: MessageRole::System,
        parts: vec![MessagePart::Text {
            text: "You are Ollama Cowork, a local-first coding assistant. The selected workspace root is represented by relative path \".\". Use only workspace-relative paths in tool calls. Prefer read-only tools until the user approves write or command capabilities. Keep answers concise and cite paths you inspected."
                .to_string(),
        }],
    };
    let user = ConversationMessage {
        id: Uuid::new_v4(),
        role: MessageRole::User,
        parts: vec![MessagePart::Text {
            text: request.user_prompt,
        }],
    };

    let mut conversation = vec![system];
    conversation.extend(ContextBuilder::default().build_model_history(request.history));
    conversation.push(user.clone());

    let mut messages = vec![user];
    let done_reason;
    let mut tool_iteration_count = 0;

    loop {
        cancellation.check()?;

        let chat_request = ChatRequest {
            model: request.model.clone(),
            messages: conversation.clone(),
            tools: request.tools.clone(),
            think: ThinkMode::Enabled(true),
        };
        let response = tokio::select! {
            response = backend.chat(chat_request) => response?,
            _ = cancellation.cancelled() => return Err(AppError::Cancelled),
        };
        let response_done_reason = response.done_reason.clone();
        let tool_calls = response.tool_calls.clone();
        let assistant = assistant_message_from_response(response);

        conversation.push(assistant.clone());
        messages.push(assistant);

        if tool_calls.is_empty() {
            done_reason = response_done_reason;
            break;
        }

        if tool_iteration_count >= MAX_TOOL_ITERATIONS {
            return Err(AppError::Runtime(format!(
                "agent exceeded maximum tool iterations ({MAX_TOOL_ITERATIONS})"
            )));
        }
        tool_iteration_count += 1;

        for call in tool_calls {
            cancellation.check()?;
            let result = match tool_registry
                .execute(
                    ToolExecutionRequest {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                    cancellation.clone(),
                )
                .await
            {
                Ok(result) => result,
                Err(err) => ToolResult {
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    content: serde_json::json!({
                        "error": err.to_string(),
                    }),
                },
            };
            let tool = ConversationMessage {
                id: Uuid::new_v4(),
                role: MessageRole::Tool,
                parts: vec![MessagePart::ToolResult { result }],
            };

            conversation.push(tool.clone());
            messages.push(tool);
        }
    }

    Ok(AgentTurnResponse {
        messages,
        done_reason,
        tool_iteration_count,
    })
}

pub fn assistant_message_from_response(response: ChatResponse) -> ConversationMessage {
    let mut parts = Vec::new();

    if let Some(thinking) = response.thinking {
        parts.push(MessagePart::Thinking { text: thinking });
    }

    if let Some(content) = response.content {
        parts.push(MessagePart::Text { text: content });
    }

    for call in response.tool_calls {
        parts.push(MessagePart::ToolCall { call });
    }

    ConversationMessage {
        id: Uuid::new_v4(),
        role: MessageRole::Assistant,
        parts,
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Mutex;

    use crate::core::messages::ToolCall;
    use crate::core::model::{ChatResponse, ModelTimings};

    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl ModelBackend for FakeBackend {
        async fn probe(&self) -> AppResult<crate::core::model::ProbeOllamaResponse> {
            unreachable!("agent tests do not probe")
        }

        async fn chat(&self, _request: ChatRequest) -> AppResult<ChatResponse> {
            let mut calls = self.calls.lock().expect("calls lock");
            *calls += 1;

            if *calls == 1 {
                return Ok(ChatResponse {
                    thinking: Some("Need to inspect files.".to_string()),
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: Some("call_1".to_string()),
                        name: "list_files".to_string(),
                        arguments: json!({ "path": "." }),
                    }],
                    done_reason: None,
                    timings: ModelTimings::default(),
                });
            }

            Ok(ChatResponse {
                thinking: None,
                content: Some("The workspace root was listed.".to_string()),
                tool_calls: Vec::new(),
                done_reason: Some("stop".to_string()),
                timings: ModelTimings::default(),
            })
        }
    }

    struct FakeTools;

    #[async_trait]
    impl ToolRegistry for FakeTools {
        async fn execute(
            &self,
            request: ToolExecutionRequest,
            cancellation: CancellationFlag,
        ) -> AppResult<ToolResult> {
            cancellation.check()?;
            Ok(ToolResult {
                call_id: request.call_id,
                name: request.name,
                content: json!({ "entries": [] }),
            })
        }
    }

    #[tokio::test]
    async fn agent_turn_runs_tools_and_returns_appendable_messages() {
        let response = run_agent_turn(
            &FakeBackend::default(),
            &FakeTools,
            AgentTurnRequest {
                model: "test".to_string(),
                history: Vec::new(),
                user_prompt: "List files".to_string(),
                tools: Vec::new(),
            },
            CancellationFlag::default(),
        )
        .await
        .expect("agent turn");

        assert_eq!(response.tool_iteration_count, 1);
        assert_eq!(response.done_reason.as_deref(), Some("stop"));
        assert!(matches!(response.messages[0].role, MessageRole::User));
        assert_eq!(response.messages.len(), 4);
    }

    #[tokio::test]
    async fn agent_turn_honors_pre_cancelled_run() {
        let cancellation = CancellationFlag::default();
        cancellation.cancel();

        let err = run_agent_turn(
            &FakeBackend::default(),
            &FakeTools,
            AgentTurnRequest {
                model: "test".to_string(),
                history: Vec::new(),
                user_prompt: "List files".to_string(),
                tools: Vec::new(),
            },
            cancellation,
        )
        .await
        .expect_err("pre-cancelled run should fail");

        assert!(matches!(err, AppError::Cancelled));
    }
}
