mod ollama;
mod types;

pub use ollama::{OllamaBackend, OllamaConfig};
pub use types::{
    ChatRequest, ChatResponse, ModelBackend, ModelInfo, ModelTimings, ProbeOllamaResponse,
    ThinkMode, ToolDefinition,
};
