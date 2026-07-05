use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use url::Url;

use crate::core::error::{AppError, AppResult};
use crate::core::messages::{MessagePart, MessageRole, ToolCall};
use crate::core::model::types::{
    ChatRequest, ChatResponse, ModelBackend, ModelInfo, ModelTimings, ProbeOllamaResponse,
};

const OLLAMA_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const OLLAMA_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const OLLAMA_CHAT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct OllamaConfig {
    base_url: Url,
}

impl OllamaConfig {
    pub fn new(base_url: String) -> AppResult<Self> {
        let base_url = Url::parse(base_url.trim())
            .map_err(|err| AppError::InvalidConfig(format!("invalid Ollama URL: {err}")))?;

        if base_url.scheme() != "http" && base_url.scheme() != "https" {
            return Err(AppError::InvalidConfig(
                "Ollama URL must use http or https".to_string(),
            ));
        }

        Ok(Self { base_url })
    }

    fn endpoint(&self, path: &str) -> AppResult<Url> {
        self.base_url
            .join(path)
            .map_err(|err| AppError::InvalidConfig(format!("invalid Ollama endpoint: {err}")))
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }
}

#[derive(Debug, Clone)]
pub struct OllamaBackend {
    config: OllamaConfig,
    client: Client,
}

impl OllamaBackend {
    pub fn new(config: OllamaConfig) -> AppResult<Self> {
        let client = Client::builder()
            .connect_timeout(OLLAMA_CONNECT_TIMEOUT)
            .build()
            .map_err(|err| AppError::ModelBackend(err.to_string()))?;

        Ok(Self { config, client })
    }
}

#[async_trait]
impl ModelBackend for OllamaBackend {
    async fn probe(&self) -> AppResult<ProbeOllamaResponse> {
        let version = self
            .client
            .get(self.config.endpoint("/api/version")?)
            .timeout(OLLAMA_PROBE_TIMEOUT)
            .send()
            .await
            .map_err(|err| AppError::ModelBackend(err.to_string()))?
            .error_for_status()
            .map_err(|err| AppError::ModelBackend(err.to_string()))?
            .json::<OllamaVersionResponse>()
            .await
            .map_err(|err| AppError::ModelBackend(err.to_string()))?;

        let tags = self
            .client
            .get(self.config.endpoint("/api/tags")?)
            .timeout(OLLAMA_PROBE_TIMEOUT)
            .send()
            .await
            .map_err(|err| AppError::ModelBackend(err.to_string()))?
            .error_for_status()
            .map_err(|err| AppError::ModelBackend(err.to_string()))?
            .json::<OllamaTagsResponse>()
            .await
            .map_err(|err| AppError::ModelBackend(err.to_string()))?;

        Ok(ProbeOllamaResponse {
            base_url: self.config.base_url().to_string(),
            version: Some(version.version),
            models: tags.models.into_iter().map(ModelInfo::from).collect(),
        })
    }

    async fn chat(&self, request: ChatRequest) -> AppResult<ChatResponse> {
        let body = json!({
            "model": request.model,
            "messages": request.messages.into_iter().map(to_ollama_message).collect::<Vec<_>>(),
            "tools": request.tools.into_iter().map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            }).collect::<Vec<_>>(),
            "think": request.think,
            "stream": false,
        });

        let response = self
            .client
            .post(self.config.endpoint("/api/chat")?)
            .timeout(OLLAMA_CHAT_TIMEOUT)
            .json(&body)
            .send()
            .await
            .map_err(|err| AppError::ModelBackend(err.to_string()))?
            .error_for_status()
            .map_err(|err| AppError::ModelBackend(err.to_string()))?
            .json::<OllamaChatResponse>()
            .await
            .map_err(|err| AppError::ModelBackend(err.to_string()))?;

        Ok(ChatResponse {
            thinking: response.message.thinking,
            content: response
                .message
                .content
                .filter(|content| !content.is_empty()),
            tool_calls: response
                .message
                .tool_calls
                .unwrap_or_default()
                .into_iter()
                .map(ToolCall::from)
                .collect(),
            done_reason: response.done_reason,
            timings: ModelTimings {
                total_duration_ns: response.total_duration,
                load_duration_ns: response.load_duration,
                prompt_eval_count: response.prompt_eval_count,
                prompt_eval_duration_ns: response.prompt_eval_duration,
                eval_count: response.eval_count,
                eval_duration_ns: response.eval_duration,
            },
        })
    }
}

pub(crate) fn to_ollama_message(message: crate::core::messages::ConversationMessage) -> Value {
    let role = match message.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    };

    let mut content = String::new();
    let mut thinking = None;
    let mut tool_calls = Vec::new();
    let mut tool_name = None;

    for part in message.parts {
        match part {
            MessagePart::Thinking { text } => thinking = Some(text),
            MessagePart::Text { text } => content.push_str(&text),
            MessagePart::ToolCall { call } => tool_calls.push(json!({
                "id": call.id,
                "function": {
                    "name": call.name,
                    "arguments": call.arguments,
                }
            })),
            MessagePart::ToolResult { result } => {
                tool_name = Some(result.name);
                content.push_str(&result.content.to_string());
            }
            MessagePart::ApprovalRequest { .. } | MessagePart::Diff { .. } => {}
        }
    }

    let mut value = json!({
        "role": role,
        "content": content,
    });

    if let Some(thinking) = thinking {
        value["thinking"] = json!(thinking);
    }

    if !tool_calls.is_empty() {
        value["tool_calls"] = json!(tool_calls);
    }

    if let Some(tool_name) = tool_name {
        value["tool_name"] = json!(tool_name);
    }

    value
}

#[derive(Debug, Deserialize)]
struct OllamaVersionResponse {
    version: String,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
    details: Option<OllamaModelDetails>,
    capabilities: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelDetails {
    family: Option<String>,
    parameter_size: Option<String>,
}

impl From<OllamaModel> for ModelInfo {
    fn from(model: OllamaModel) -> Self {
        Self {
            name: model.name,
            family: model
                .details
                .as_ref()
                .and_then(|details| details.family.clone()),
            parameter_size: model
                .details
                .as_ref()
                .and_then(|details| details.parameter_size.clone()),
            capabilities: model.capabilities.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
    done_reason: Option<String>,
    total_duration: Option<u64>,
    load_duration: Option<u64>,
    prompt_eval_count: Option<u64>,
    prompt_eval_duration: Option<u64>,
    eval_count: Option<u64>,
    eval_duration: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    content: Option<String>,
    thinking: Option<String>,
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OllamaToolCall {
    id: Option<String>,
    function: OllamaToolFunction,
}

#[derive(Debug, Deserialize)]
struct OllamaToolFunction {
    name: String,
    arguments: Value,
}

impl From<OllamaToolCall> for ToolCall {
    fn from(call: OllamaToolCall) -> Self {
        Self {
            id: call.id,
            name: call.function.name,
            arguments: call.function.arguments,
        }
    }
}
