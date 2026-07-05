use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::core::messages::{ConversationMessage, MessagePart, MessageRole, ToolCall, ToolResult};
use crate::core::model::{
    ChatRequest, ChatResponse, ModelBackend, OllamaBackend, OllamaConfig, ProbeOllamaResponse,
    ThinkMode,
};
use crate::core::tools::{LocalToolRegistry, ToolExecutionRequest, ToolRegistry};
use crate::core::workspace::{WorkspaceContext, WorkspaceSelection, WorkspaceSelectionStore};

#[tauri::command]
pub async fn probe_ollama(base_url: String) -> Result<ProbeOllamaResponse, String> {
    let config = OllamaConfig::new(base_url).map_err(|err| err.to_string())?;
    let backend = OllamaBackend::new(config).map_err(|err| err.to_string())?;
    backend.probe().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn choose_workspace(
    app: AppHandle,
    selections: State<'_, WorkspaceSelectionStore>,
) -> Result<Option<WorkspaceSelection>, String> {
    let Some(folder) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };

    let path = folder
        .into_path()
        .map_err(|err| format!("selected workspace path could not be resolved: {err}"))?;
    let workspace = WorkspaceContext::new(path).map_err(|err| err.to_string())?;
    selections
        .insert(workspace)
        .map(Some)
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn run_tool_probe(
    base_url: String,
    model: String,
    workspace_id: Uuid,
    selections: State<'_, WorkspaceSelectionStore>,
) -> Result<ToolProbeResponse, String> {
    let config = OllamaConfig::new(base_url).map_err(|err| err.to_string())?;
    let backend = OllamaBackend::new(config).map_err(|err| err.to_string())?;
    let workspace = selections
        .get(workspace_id)
        .map_err(|err| err.to_string())?;
    let tools = LocalToolRegistry::new(workspace.clone());
    let tool_definitions = tools.definitions();

    let system = ConversationMessage {
        id: Uuid::new_v4(),
        role: MessageRole::System,
        parts: vec![MessagePart::Text {
            text: "You are a tool-calling agent. The selected workspace root is represented by relative path \".\". Use only workspace-relative paths in tool calls. When asked to list the repository root, call list_files with path exactly \".\". After the tool result, summarize briefly.".to_string(),
        }],
    };

    let user = ConversationMessage {
        id: Uuid::new_v4(),
        role: MessageRole::User,
        parts: vec![MessagePart::Text {
            text: "List the repository root with the available tool, then summarize it."
                .to_string(),
        }],
    };

    let first = backend
        .chat(ChatRequest {
            model: model.clone(),
            messages: vec![system.clone(), user.clone()],
            tools: tool_definitions.clone(),
            think: ThinkMode::Enabled(true),
        })
        .await
        .map_err(|err| err.to_string())?;

    let tool_call = first
        .tool_calls
        .first()
        .cloned()
        .ok_or_else(|| "model did not return a tool call".to_string())?;

    if tool_call.name != "list_files" {
        return Err(format!("unexpected tool call: {}", tool_call.name));
    }

    let tool_result = tools
        .execute(ToolExecutionRequest {
            call_id: tool_call.id.clone(),
            name: tool_call.name.clone(),
            arguments: tool_call.arguments.clone(),
        })
        .await
        .map_err(|err| err.to_string())?;
    let assistant = assistant_message_from_response(first.clone());
    let tool = ConversationMessage {
        id: Uuid::new_v4(),
        role: MessageRole::Tool,
        parts: vec![MessagePart::ToolResult {
            result: tool_result.clone(),
        }],
    };

    let final_response = backend
        .chat(ChatRequest {
            model,
            messages: vec![system, user, assistant, tool],
            tools: tool_definitions,
            think: ThinkMode::Enabled(true),
        })
        .await
        .map_err(|err| err.to_string())?;

    Ok(ToolProbeResponse {
        first_thinking: first.thinking,
        tool_call,
        tool_result,
        final_thinking: final_response.thinking,
        final_content: final_response.content.unwrap_or_default(),
        done_reason: final_response.done_reason,
    })
}

fn assistant_message_from_response(response: ChatResponse) -> ConversationMessage {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolProbeResponse {
    pub first_thinking: Option<String>,
    pub tool_call: ToolCall,
    pub tool_result: ToolResult,
    pub final_thinking: Option<String>,
    pub final_content: String,
    pub done_reason: Option<String>,
}
