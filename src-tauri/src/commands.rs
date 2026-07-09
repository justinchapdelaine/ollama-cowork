use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::core::agent::{
    assistant_message_from_response, run_agent_turn as run_agent_turn_core,
    run_agent_turn_streaming as run_agent_turn_streaming_core, AgentRunEvent, AgentTurnRequest,
    AgentTurnResponse,
};
use crate::core::approval::{
    submit_approval_request, ApprovalRequest, ApprovalRequestStore, ApprovalSubmission,
    DefaultApprovalPolicy, PendingApproval, ResolvedApproval,
};
use crate::core::messages::{ConversationMessage, MessagePart, MessageRole, ToolCall, ToolResult};
use crate::core::model::{
    ChatRequest, ModelBackend, OllamaBackend, OllamaConfig, ProbeOllamaResponse, ThinkMode,
};
use crate::core::run::{AgentRunStore, CancellationFlag};
use crate::core::runtime::{
    CommandSpec, HostCommandRunner, QueuedRuntimeCommand, RuntimeCommandQueue,
    RuntimeCommandResolution, RuntimeCommandRunner,
};
use crate::core::session::{
    CreateSessionRequest, JsonlSessionStore, SessionEvent, SessionId, SessionSnapshot,
    SessionStore, SessionSummary,
};
use crate::core::tools::{
    LocalToolRegistry, PolicyEnforcedToolRegistry, ToolExecutionRequest, ToolRegistry,
};
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
    let policy_tools = PolicyEnforcedToolRegistry::with_default_policy(tools);

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

    let tool_result = policy_tools
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
    let tool_definitions = tools.definitions();
    let policy_tools = PolicyEnforcedToolRegistry::with_default_policy(tools);
    let cancellation = runs.begin(request.run_id).map_err(|err| err.to_string())?;

    let result = run_agent_turn_core(
        &backend,
        &policy_tools,
        AgentTurnRequest {
            model: request.model,
            history: request.history,
            user_prompt: request.user_prompt,
            tools: tool_definitions,
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
    let tool_definitions = tools.definitions();
    let policy_tools = PolicyEnforcedToolRegistry::with_default_policy(tools);
    let cancellation = runs.begin(request.run_id).map_err(|err| err.to_string())?;
    let run_id = request.run_id;
    let mut emit_event = |event: AgentRunEvent| {
        app.emit(AGENT_RUN_EVENT, AgentRunEventEnvelope { run_id, event })
            .map_err(|err| crate::core::error::AppError::Runtime(err.to_string()))
    };

    let result = run_agent_turn_streaming_core(
        &backend,
        &policy_tools,
        AgentTurnRequest {
            model: request.model,
            history: request.history,
            user_prompt: request.user_prompt,
            tools: tool_definitions,
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

#[tauri::command]
pub async fn list_pending_approvals(
    approvals: State<'_, ApprovalRequestStore>,
) -> Result<Vec<PendingApproval>, String> {
    approvals.list().map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn request_runtime_command_approval(
    request: RuntimeCommandApprovalRequest,
    approvals: State<'_, ApprovalRequestStore>,
    queued_commands: State<'_, RuntimeCommandQueue>,
    command_runner: State<'_, HostCommandRunner>,
    sessions: State<'_, JsonlSessionStore>,
    selections: State<'_, WorkspaceSelectionStore>,
) -> Result<RuntimeCommandApprovalSubmission, String> {
    request_runtime_command_approval_core(
        request,
        &approvals,
        &queued_commands,
        &sessions,
        &selections,
        &*command_runner,
        &DefaultApprovalPolicy,
    )
    .await
    .map_err(|err| err.to_string())
}

async fn request_runtime_command_approval_core<P, R>(
    request: RuntimeCommandApprovalRequest,
    approvals: &ApprovalRequestStore,
    queued_commands: &RuntimeCommandQueue,
    sessions: &JsonlSessionStore,
    selections: &WorkspaceSelectionStore,
    command_runner: &R,
    policy: &P,
) -> crate::core::error::AppResult<RuntimeCommandApprovalSubmission>
where
    P: crate::core::approval::ApprovalPolicy,
    R: RuntimeCommandRunner,
{
    let workspace = selections.get(request.workspace_id)?;
    let resolved_cwd = workspace.resolve_existing_relative_path(&request.command.cwd.0)?;
    if !resolved_cwd.is_dir() {
        return Err(crate::core::error::AppError::InvalidConfig(format!(
            "runtime command cwd is not a directory: {}",
            request.command.cwd.display()
        )));
    }

    let approval_request = request.command.approval_request();
    let request_id = approval_request.id;
    let queued = QueuedRuntimeCommand {
        request_id,
        session_id: request.session_id,
        run_id: request.run_id,
        workspace_id: request.workspace_id,
        workspace_root: workspace.source_root().to_path_buf(),
        resolved_cwd,
        command: request.command,
    };
    queued_commands.insert(queued)?;

    let submission = submit_approval_request(
        approval_request,
        Some(request.session_id),
        request.run_id,
        policy,
        approvals,
        Some(sessions),
    )
    .await;

    match submission {
        Ok(ApprovalSubmission::Allowed { request }) => {
            let runtime_command = if let Some(command) = queued_commands.take(request_id)? {
                Some(execute_runtime_command(command, command_runner, sessions).await?)
            } else {
                None
            };
            Ok(RuntimeCommandApprovalSubmission::Allowed {
                request,
                runtime_command,
            })
        }
        Ok(ApprovalSubmission::PendingManualApproval { pending }) => {
            Ok(RuntimeCommandApprovalSubmission::PendingManualApproval { pending })
        }
        Err(err) => {
            let _ = queued_commands.remove(request_id);
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn resolve_approval(
    request_id: Uuid,
    approved: bool,
    reason: String,
    approvals: State<'_, ApprovalRequestStore>,
    queued_commands: State<'_, RuntimeCommandQueue>,
    command_runner: State<'_, HostCommandRunner>,
    sessions: State<'_, JsonlSessionStore>,
) -> Result<ApprovalResolutionResponse, String> {
    resolve_approval_core(
        request_id,
        approved,
        reason,
        &approvals,
        &queued_commands,
        &*command_runner,
        &sessions,
    )
    .await
    .map_err(|err| err.to_string())
}

async fn resolve_approval_core<R>(
    request_id: Uuid,
    approved: bool,
    reason: String,
    approvals: &ApprovalRequestStore,
    queued_commands: &RuntimeCommandQueue,
    command_runner: &R,
    sessions: &JsonlSessionStore,
) -> crate::core::error::AppResult<ApprovalResolutionResponse>
where
    R: RuntimeCommandRunner,
{
    let resolved = approvals.resolve(request_id, approved, "user", reason)?;
    let queued_command = queued_commands.take(request_id)?;

    if let Some(session_id) = resolved.session_id {
        sessions
            .append_event(
                session_id,
                SessionEvent::ApprovalResolved {
                    run_id: resolved.run_id,
                    decision: resolved.decision.clone(),
                },
            )
            .await?;
    }

    let runtime_command = if resolved.decision.approved {
        if let Some(command) = queued_command {
            Some(execute_runtime_command(command, command_runner, sessions).await?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(ApprovalResolutionResponse {
        approval: resolved,
        runtime_command,
    })
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
pub struct RuntimeCommandApprovalRequest {
    pub session_id: SessionId,
    pub run_id: Option<Uuid>,
    pub workspace_id: Uuid,
    pub command: CommandSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeCommandApprovalSubmission {
    Allowed {
        request: ApprovalRequest,
        runtime_command: Option<RuntimeCommandResolution>,
    },
    PendingManualApproval {
        pending: PendingApproval,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolutionResponse {
    #[serde(flatten)]
    pub approval: ResolvedApproval,
    pub runtime_command: Option<RuntimeCommandResolution>,
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

async fn execute_runtime_command(
    command: QueuedRuntimeCommand,
    runner: &impl RuntimeCommandRunner,
    sessions: &JsonlSessionStore,
) -> crate::core::error::AppResult<RuntimeCommandResolution> {
    let message_id = Uuid::new_v4();
    let request_id = command.request_id;
    let session_id = command.session_id;
    let run_id = command.run_id;
    let command_spec = command.command.clone();

    match runner.run(&command).await {
        Ok(result) => {
            sessions
                .append_event(
                    session_id,
                    SessionEvent::RuntimeCommandCompleted {
                        message_id,
                        run_id,
                        request_id,
                        command: command_spec.clone(),
                        result: result.clone(),
                    },
                )
                .await?;
            Ok(RuntimeCommandResolution::Completed {
                message_id,
                request_id,
                command: command_spec,
                result,
            })
        }
        Err(err) => {
            let message = err.to_string();
            sessions
                .append_event(
                    session_id,
                    SessionEvent::RuntimeCommandFailed {
                        message_id,
                        run_id,
                        request_id,
                        command: command_spec.clone(),
                        message: message.clone(),
                    },
                )
                .await?;
            Ok(RuntimeCommandResolution::Failed {
                message_id,
                request_id,
                command: command_spec,
                message,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::approval::{ApprovalPolicy, ApprovalRequirement};
    use crate::core::runtime::{CommandResult, NetworkPolicy, RelativePath};
    use crate::core::session::TimestampedSessionEvent;
    use crate::core::workspace::WorkspaceSelectionStore;
    use async_trait::async_trait;
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    #[tokio::test]
    async fn approved_runtime_command_runs_once_and_is_logged() {
        let fixture = RuntimeApprovalFixture::new().await;
        let submitted = fixture
            .request(DefaultApprovalPolicy)
            .await
            .expect("request approval");
        let request_id = match submitted {
            RuntimeCommandApprovalSubmission::PendingManualApproval { pending } => {
                pending.request.id
            }
            RuntimeCommandApprovalSubmission::Allowed { .. } => {
                panic!("default command policy should require manual approval")
            }
        };

        let resolved = resolve_approval_core(
            request_id,
            true,
            "approved".to_string(),
            &fixture.approvals,
            &fixture.queued_commands,
            &fixture.runner,
            &fixture.sessions,
        )
        .await
        .expect("resolve approval");

        assert!(resolved.approval.decision.approved);
        assert!(matches!(
            resolved.runtime_command,
            Some(RuntimeCommandResolution::Completed { request_id: resolved_request_id, .. })
                if resolved_request_id == request_id
        ));
        assert_eq!(fixture.runner.calls(), 1);
        assert!(fixture.approvals.list().expect("list approvals").is_empty());

        let session = fixture
            .sessions
            .load_session(fixture.session_id)
            .await
            .expect("load session");
        assert!(matches!(
            session.events.as_slice(),
            [
                TimestampedSessionEvent {
                    event: SessionEvent::ApprovalRequested { .. },
                    ..
                },
                TimestampedSessionEvent {
                    event: SessionEvent::ApprovalResolved { .. },
                    ..
                },
                TimestampedSessionEvent {
                    event: SessionEvent::RuntimeCommandCompleted { .. },
                    ..
                },
            ]
        ));
    }

    #[tokio::test]
    async fn denied_runtime_command_is_not_executed() {
        let fixture = RuntimeApprovalFixture::new().await;
        let submitted = fixture
            .request(DefaultApprovalPolicy)
            .await
            .expect("request approval");
        let request_id = match submitted {
            RuntimeCommandApprovalSubmission::PendingManualApproval { pending } => {
                pending.request.id
            }
            RuntimeCommandApprovalSubmission::Allowed { .. } => {
                panic!("default command policy should require manual approval")
            }
        };

        let resolved = resolve_approval_core(
            request_id,
            false,
            "denied".to_string(),
            &fixture.approvals,
            &fixture.queued_commands,
            &fixture.runner,
            &fixture.sessions,
        )
        .await
        .expect("resolve approval");

        assert!(!resolved.approval.decision.approved);
        assert!(resolved.runtime_command.is_none());
        assert_eq!(fixture.runner.calls(), 0);
        assert!(fixture.approvals.list().expect("list approvals").is_empty());
        assert!(fixture
            .queued_commands
            .take(request_id)
            .expect("take queued command")
            .is_none());
    }

    #[tokio::test]
    async fn duplicate_runtime_approval_resolution_does_not_log_second_decision() {
        let fixture = RuntimeApprovalFixture::new().await;
        let submitted = fixture
            .request(DefaultApprovalPolicy)
            .await
            .expect("request approval");
        let request_id = match submitted {
            RuntimeCommandApprovalSubmission::PendingManualApproval { pending } => {
                pending.request.id
            }
            RuntimeCommandApprovalSubmission::Allowed { .. } => {
                panic!("default command policy should require manual approval")
            }
        };

        resolve_approval_core(
            request_id,
            true,
            "approved".to_string(),
            &fixture.approvals,
            &fixture.queued_commands,
            &fixture.runner,
            &fixture.sessions,
        )
        .await
        .expect("resolve first approval");
        let err = resolve_approval_core(
            request_id,
            true,
            "approved again".to_string(),
            &fixture.approvals,
            &fixture.queued_commands,
            &fixture.runner,
            &fixture.sessions,
        )
        .await
        .expect_err("second resolution should fail");

        let session = fixture
            .sessions
            .load_session(fixture.session_id)
            .await
            .expect("load session");
        let approval_resolved_count = session
            .events
            .iter()
            .filter(|event| matches!(event.event, SessionEvent::ApprovalResolved { .. }))
            .count();

        assert!(err.to_string().contains("unknown approval request"));
        assert_eq!(approval_resolved_count, 1);
        assert_eq!(fixture.runner.calls(), 1);
    }

    #[tokio::test]
    async fn auto_allowed_runtime_command_executes_and_logs_immediately() {
        let fixture = RuntimeApprovalFixture::new().await;
        let submitted = fixture
            .request(AllowAllPolicy)
            .await
            .expect("request approval");

        assert!(matches!(
            submitted,
            RuntimeCommandApprovalSubmission::Allowed {
                runtime_command: Some(RuntimeCommandResolution::Completed { .. }),
                ..
            }
        ));
        assert_eq!(fixture.runner.calls(), 1);
        assert!(fixture.approvals.list().expect("list approvals").is_empty());

        let session = fixture
            .sessions
            .load_session(fixture.session_id)
            .await
            .expect("load session");
        assert!(matches!(
            session.events.as_slice(),
            [TimestampedSessionEvent {
                event: SessionEvent::RuntimeCommandCompleted { .. },
                ..
            }]
        ));
    }

    struct RuntimeApprovalFixture {
        approvals: ApprovalRequestStore,
        queued_commands: RuntimeCommandQueue,
        sessions: JsonlSessionStore,
        selections: WorkspaceSelectionStore,
        runner: RecordingRunner,
        session_id: SessionId,
        workspace_id: Uuid,
    }

    impl RuntimeApprovalFixture {
        async fn new() -> Self {
            let workspace_root = test_dir("runtime-command-workspace");
            fs::create_dir_all(&workspace_root).expect("create workspace");
            let selections = WorkspaceSelectionStore::default();
            let workspace = WorkspaceContext::new(&workspace_root).expect("workspace context");
            let selection = selections.insert(workspace).expect("insert workspace");
            let sessions = JsonlSessionStore::new(test_dir("runtime-command-sessions"));
            let session = sessions
                .create_session(CreateSessionRequest {
                    title: "Runtime command".to_string(),
                    workspace_root: Some(workspace_root.display().to_string()),
                    base_url: None,
                    model: None,
                })
                .await
                .expect("create session");

            Self {
                approvals: ApprovalRequestStore::default(),
                queued_commands: RuntimeCommandQueue::default(),
                sessions,
                selections,
                runner: RecordingRunner::default(),
                session_id: session.id,
                workspace_id: selection.id,
            }
        }

        async fn request<P>(
            &self,
            policy: P,
        ) -> crate::core::error::AppResult<RuntimeCommandApprovalSubmission>
        where
            P: ApprovalPolicy,
        {
            request_runtime_command_approval_core(
                RuntimeCommandApprovalRequest {
                    session_id: self.session_id,
                    run_id: Some(Uuid::new_v4()),
                    workspace_id: self.workspace_id,
                    command: CommandSpec {
                        program: "test-command".to_string(),
                        args: vec!["--flag".to_string()],
                        cwd: RelativePath(PathBuf::from(".")),
                        timeout_ms: 1_000,
                        network: NetworkPolicy::Offline,
                    },
                },
                &self.approvals,
                &self.queued_commands,
                &self.sessions,
                &self.selections,
                &self.runner,
                &policy,
            )
            .await
        }
    }

    #[derive(Debug, Default, Clone)]
    struct RecordingRunner {
        commands: Arc<Mutex<Vec<QueuedRuntimeCommand>>>,
    }

    impl RecordingRunner {
        fn calls(&self) -> usize {
            self.commands.lock().expect("runner lock").len()
        }
    }

    #[async_trait]
    impl RuntimeCommandRunner for RecordingRunner {
        async fn run(
            &self,
            command: &QueuedRuntimeCommand,
        ) -> crate::core::error::AppResult<CommandResult> {
            self.commands
                .lock()
                .expect("runner lock")
                .push(command.clone());
            Ok(CommandResult {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                duration_ms: 5,
                timed_out: false,
            })
        }
    }

    #[derive(Debug, Clone)]
    struct AllowAllPolicy;

    impl ApprovalPolicy for AllowAllPolicy {
        fn evaluate(&self, _request: &ApprovalRequest) -> ApprovalRequirement {
            ApprovalRequirement::Allow
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join("ollama-cowork-command-tests")
            .join(name)
            .join(Uuid::new_v4().to_string())
    }
}
