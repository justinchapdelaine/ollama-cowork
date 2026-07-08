use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: Uuid,
    pub role: MessageRole,
    pub parts: Vec<MessagePart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
    Thinking { text: String },
    Text { text: String },
    ToolCall { call: ToolCall },
    ToolResult { result: ToolResult },
    ApprovalRequest { request: ApprovalRequestMessage },
    ApprovalDecision { decision: ApprovalDecisionMessage },
    Diff { diff: DiffMessage },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: Option<String>,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: Option<String>,
    pub name: String,
    pub content: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestMessage {
    pub id: Uuid,
    pub summary: String,
    pub requested_capabilities: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionMessage {
    pub id: Uuid,
    pub request_id: Uuid,
    pub approved: bool,
    pub reviewer: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffMessage {
    pub summary: String,
    pub patch: String,
}
