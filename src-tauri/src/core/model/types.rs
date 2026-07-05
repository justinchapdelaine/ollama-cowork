use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::error::AppResult;
use crate::core::messages::{ConversationMessage, ToolCall};

#[async_trait]
pub trait ModelBackend: Send + Sync {
    async fn probe(&self) -> AppResult<ProbeOllamaResponse>;
    async fn chat(&self, request: ChatRequest) -> AppResult<ChatResponse>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ConversationMessage>,
    pub tools: Vec<ToolDefinition>,
    pub think: ThinkMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThinkMode {
    Enabled(bool),
    Level(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub thinking: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub done_reason: Option<String>,
    pub timings: ModelTimings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelTimings {
    pub total_duration_ns: Option<u64>,
    pub load_duration_ns: Option<u64>,
    pub prompt_eval_count: Option<u64>,
    pub prompt_eval_duration_ns: Option<u64>,
    pub eval_count: Option<u64>,
    pub eval_duration_ns: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeOllamaResponse {
    pub base_url: String,
    pub version: Option<String>,
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub capabilities: Vec<String>,
}
