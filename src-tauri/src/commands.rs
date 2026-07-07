use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::core::agent::{
    assistant_message_from_response, run_agent_turn as run_agent_turn_core,
    run_agent_turn_streaming as run_agent_turn_streaming_core, AgentRunEvent, AgentTurnRequest,
    AgentTurnResponse,
};
use crate::core::messages::{ConversationMessage, MessagePart, MessageRole, ToolCall, ToolResult};
use crate::core::model::{
    ChatRequest, ModelBackend, OllamaBackend, OllamaConfig, ProbeOllamaResponse, ThinkMode,
};
use crate::core::run::{AgentRunStore, CancellationFlag};
use crate::core::session::{
    CreateSessionRequest, JsonlSessionStore, SessionEvent, SessionId, SessionSnapshot,
    SessionStore, SessionSummary,
};
use crate::core::tools::{LocalToolRegistry, ToolExecutionRequest, ToolRegistry};
use crate::core::workspace::{WorkspaceContext, WorkspaceSelection, WorkspaceSelectionStore};

const AGENT_RUN_EVENT: &str = "agent-run-event";

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
pub async fn select_workspace_path(
    source_root: String,
    selections: State<'_, WorkspaceSelectionStore>,
) -> Result<WorkspaceSelection, String> {
    let workspace = WorkspaceContext::new(source_root).map_err(|err| err.to_string())?;
    selections.insert(workspace).map_err(|err| err.to_string())
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
        .execute(
            ToolExecutionRequest {
                call_id: tool_call.id.clone(),
                name: tool_call.name.clone(),
                arguments: tool_call.arguments.clone(),
            },
            CancellationFlag::default(),
        )
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

#[tauri::command]
pub async fn run_agent_turn(
    request: AgentTurnCommandRequest,
    selections: State<'_, WorkspaceSelectionStore>,
    runs: State<'_, AgentRunStore>,
) -> Result<AgentTurnResponse, String> {
    let config = OllamaConfig::new(request.base_url).map_err(|err| err.to_string())?;
    let backend = OllamaBackend::new(config).map_err(|err| err.to_string())?;
    let workspace = selections
        .get(request.workspace_id)
        .map_err(|err| err.to_string())?;
    let tools = LocalToolRegistry::new(workspace);
    let cancellation = runs.begin(request.run_id).map_err(|err| err.to_string())?;

    let result = run_agent_turn_core(
        &backend,
        &tools,
        AgentTurnRequest {
            model: request.model,
            history: request.history,
            user_prompt: request.user_prompt,
            tools: tools.definitions(),
        },
        cancellation,
    )
    .await;
    let finish_result = runs.finish(request.run_id);

    match (result, finish_result) {
        (Ok(response), Ok(())) => Ok(response),
        (Err(err), _) => Err(err.to_string()),
        (Ok(_), Err(err)) => Err(err.to_string()),
    }
}

#[tauri::command]
pub async fn run_agent_turn_stream(
    app: AppHandle,
    request: AgentTurnCommandRequest,
    selections: State<'_, WorkspaceSelectionStore>,
    runs: State<'_, AgentRunStore>,
) -> Result<AgentTurnResponse, String> {
    let config = OllamaConfig::new(request.base_url).map_err(|err| err.to_string())?;
    let backend = OllamaBackend::new(config).map_err(|err| err.to_string())?;
    let workspace = selections
        .get(request.workspace_id)
        .map_err(|err| err.to_string())?;
    let tools = LocalToolRegistry::new(workspace);
    let cancellation = runs.begin(request.run_id).map_err(|err| err.to_string())?;
    let run_id = request.run_id;
    let mut emit_event = |event: AgentRunEvent| {
        app.emit(AGENT_RUN_EVENT, AgentRunEventEnvelope { run_id, event })
            .map_err(|err| crate::core::error::AppError::Runtime(err.to_string()))
    };

    let result = run_agent_turn_streaming_core(
        &backend,
        &tools,
        AgentTurnRequest {
            model: request.model,
            history: request.history,
            user_prompt: request.user_prompt,
            tools: tools.definitions(),
        },
        cancellation,
        &mut emit_event,
    )
    .await;
    let finish_result = runs.finish(run_id);

    match (&result, &finish_result) {
        (Err(crate::core::error::AppError::Cancelled), _) => {
            let _ = emit_event(AgentRunEvent::Cancelled);
        }
        (Err(err), _) => {
            let _ = emit_event(AgentRunEvent::Error {
                message: err.to_string(),
            });
        }
        (Ok(_), Err(err)) => {
            let _ = emit_event(AgentRunEvent::Error {
                message: err.to_string(),
            });
        }
        (Ok(_), Ok(())) => {}
    }

    match (result, finish_result) {
        (Ok(response), Ok(())) => Ok(response),
        (Err(err), _) => Err(err.to_string()),
        (Ok(_), Err(err)) => Err(err.to_string()),
    }
}

#[tauri::command]
pub async fn cancel_agent_run(
    run_id: Uuid,
    runs: State<'_, AgentRunStore>,
) -> Result<bool, String> {
    runs.cancel(run_id).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn create_session(
    request: CreateSessionRequest,
    sessions: State<'_, JsonlSessionStore>,
) -> Result<SessionSnapshot, String> {
    sessions
        .create_session(request)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn append_session_event(
    session_id: SessionId,
    event: SessionEvent,
    sessions: State<'_, JsonlSessionStore>,
) -> Result<(), String> {
    sessions
        .append_event(session_id, event)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn load_session(
    session_id: SessionId,
    sessions: State<'_, JsonlSessionStore>,
) -> Result<SessionSnapshot, String> {
    sessions
        .load_session(session_id)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn list_sessions(
    sessions: State<'_, JsonlSessionStore>,
) -> Result<Vec<SessionSummary>, String> {
    sessions
        .list_sessions()
        .await
        .map_err(|err| err.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnCommandRequest {
    pub base_url: String,
    pub model: String,
    pub workspace_id: Uuid,
    pub run_id: Uuid,
    pub user_prompt: String,
    pub history: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunEventEnvelope {
    pub run_id: Uuid,
    pub event: AgentRunEvent,
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
