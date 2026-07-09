use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{net::IpAddr, time::Duration};
use url::{Host, Url};

use crate::core::error::{AppError, AppResult};
use crate::core::messages::{MessagePart, MessageRole, ToolCall};
use crate::core::model::types::{
    ChatRequest, ChatResponse, ChatStreamCallback, ChatStreamEvent, ModelBackend, ModelInfo,
    ModelTimings, ProbeOllamaResponse,
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

        validate_ollama_base_url_policy(&base_url)?;

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

fn validate_ollama_base_url_policy(base_url: &Url) -> AppResult<()> {
    if base_url.username() != "" || base_url.password().is_some() {
        return Err(AppError::PolicyDenied(
            "Ollama URL must not include credentials".to_string(),
        ));
    }

    let Some(host) = base_url.host() else {
        return Err(AppError::InvalidConfig(
            "Ollama URL must include a host".to_string(),
        ));
    };

    match host {
        Host::Domain(domain) if domain.eq_ignore_ascii_case("localhost") => Ok(()),
        Host::Domain(_) => Err(AppError::PolicyDenied(
            "Ollama URL host must be localhost or a private IP address".to_string(),
        )),
        Host::Ipv4(ip) if is_allowed_ollama_ip(IpAddr::V4(ip)) => Ok(()),
        Host::Ipv4(ip) => Err(AppError::PolicyDenied(format!(
            "Ollama URL host is outside the allowed local/private address ranges: {ip}"
        ))),
        Host::Ipv6(ip) if is_allowed_ollama_ip(IpAddr::V6(ip)) => Ok(()),
        Host::Ipv6(ip) => Err(AppError::PolicyDenied(format!(
            "Ollama URL host is outside the allowed local/private address ranges: {ip}"
        ))),
    }
}

fn is_allowed_ollama_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1])
        }
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
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

    async fn chat_stream(
        &self,
        request: ChatRequest,
        on_event: &mut ChatStreamCallback<'_>,
    ) -> AppResult<ChatResponse> {
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
            "stream": true,
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
            .map_err(|err| AppError::ModelBackend(err.to_string()))?;

        let mut aggregate = ChatResponseBuilder::default();
        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| AppError::ModelBackend(err.to_string()))?;
            buffer.extend_from_slice(&chunk);

            while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
                let line = buffer.drain(..=position).collect::<Vec<_>>();
                parse_stream_line(&line, &mut aggregate, on_event)?;
            }
        }

        if !buffer.is_empty() {
            parse_stream_line(&buffer, &mut aggregate, on_event)?;
        }

        if !aggregate.is_done() {
            return Err(AppError::ModelBackend(
                "Ollama stream ended before terminal done chunk".to_string(),
            ));
        }

        Ok(aggregate.build())
    }
}

#[derive(Debug, Default)]
struct ChatResponseBuilder {
    thinking: String,
    content: String,
    tool_calls: Vec<ToolCall>,
    done: bool,
    done_reason: Option<String>,
    timings: ModelTimings,
}

impl ChatResponseBuilder {
    fn apply_chunk(
        &mut self,
        chunk: OllamaChatResponse,
        on_event: &mut ChatStreamCallback<'_>,
    ) -> AppResult<()> {
        if let Some(thinking) = chunk
            .message
            .thinking
            .filter(|thinking| !thinking.is_empty())
        {
            self.thinking.push_str(&thinking);
            on_event(ChatStreamEvent::ThinkingDelta { text: thinking })?;
        }

        if let Some(content) = chunk.message.content.filter(|content| !content.is_empty()) {
            self.content.push_str(&content);
            on_event(ChatStreamEvent::ContentDelta { text: content })?;
        }

        for call in chunk.message.tool_calls.unwrap_or_default() {
            let call = ToolCall::from(call);
            self.tool_calls.push(call.clone());
            on_event(ChatStreamEvent::ToolCall { call })?;
        }

        if chunk.done.unwrap_or(false) {
            self.done = true;
            self.done_reason = chunk.done_reason;
            self.timings = ModelTimings {
                total_duration_ns: chunk.total_duration,
                load_duration_ns: chunk.load_duration,
                prompt_eval_count: chunk.prompt_eval_count,
                prompt_eval_duration_ns: chunk.prompt_eval_duration,
                eval_count: chunk.eval_count,
                eval_duration_ns: chunk.eval_duration,
            };
        }

        Ok(())
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn build(self) -> ChatResponse {
        ChatResponse {
            thinking: (!self.thinking.is_empty()).then_some(self.thinking),
            content: (!self.content.is_empty()).then_some(self.content),
            tool_calls: self.tool_calls,
            done_reason: self.done_reason,
            timings: self.timings,
        }
    }
}

fn parse_stream_line(
    line: &[u8],
    aggregate: &mut ChatResponseBuilder,
    on_event: &mut ChatStreamCallback<'_>,
) -> AppResult<()> {
    let line = String::from_utf8_lossy(line);
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let chunk = serde_json::from_str::<OllamaChatResponse>(trimmed)
        .map_err(|err| AppError::ModelBackend(format!("invalid Ollama stream chunk: {err}")))?;
    aggregate.apply_chunk(chunk, on_event)
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
            MessagePart::ApprovalRequest { .. }
            | MessagePart::ApprovalDecision { .. }
            | MessagePart::RuntimeCommandResult { .. }
            | MessagePart::RuntimeCommandError { .. }
            | MessagePart::Diff { .. } => {}
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
    done: Option<bool>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_config_accepts_localhost() {
        assert!(OllamaConfig::new("http://localhost:11434".to_string()).is_ok());
        assert!(OllamaConfig::new("http://127.0.0.1:11434".to_string()).is_ok());
        assert!(OllamaConfig::new("http://[::1]:11434".to_string()).is_ok());
    }

    #[test]
    fn ollama_config_accepts_private_lan_ips() {
        assert!(OllamaConfig::new("http://192.168.1.2:11434".to_string()).is_ok());
        assert!(OllamaConfig::new("http://10.0.0.2:11434".to_string()).is_ok());
        assert!(OllamaConfig::new("http://172.16.0.2:11434".to_string()).is_ok());
    }

    #[test]
    fn ollama_config_rejects_public_or_named_hosts() {
        assert!(OllamaConfig::new("https://example.com:11434".to_string()).is_err());
        assert!(OllamaConfig::new("http://8.8.8.8:11434".to_string()).is_err());
        assert!(OllamaConfig::new("http://user:pass@127.0.0.1:11434".to_string()).is_err());
    }

    #[test]
    fn stream_parser_aggregates_deltas_and_emits_events() {
        let mut aggregate = ChatResponseBuilder::default();
        let mut events = Vec::new();
        let mut event_sink = |event| {
            events.push(event);
            Ok(())
        };

        parse_stream_line(
            br#"{"message":{"thinking":"Need files. "},"done":false}"#,
            &mut aggregate,
            &mut event_sink,
        )
        .expect("thinking chunk");
        assert!(!aggregate.is_done());

        parse_stream_line(
            br#"{"message":{"content":"Done."},"done":false}"#,
            &mut aggregate,
            &mut event_sink,
        )
        .expect("content chunk");
        assert!(!aggregate.is_done());

        parse_stream_line(
            br#"{"message":{"content":""},"done":true,"done_reason":"stop","eval_count":3}"#,
            &mut aggregate,
            &mut event_sink,
        )
        .expect("done chunk");
        assert!(aggregate.is_done());

        let response = aggregate.build();

        assert_eq!(response.thinking.as_deref(), Some("Need files. "));
        assert_eq!(response.content.as_deref(), Some("Done."));
        assert_eq!(response.done_reason.as_deref(), Some("stop"));
        assert_eq!(response.timings.eval_count, Some(3));
        assert!(matches!(
            events.as_slice(),
            [
                ChatStreamEvent::ThinkingDelta { .. },
                ChatStreamEvent::ContentDelta { .. }
            ]
        ));
    }
}
