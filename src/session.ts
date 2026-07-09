import { invoke } from "@tauri-apps/api/core";

export type MessageRole = "system" | "user" | "assistant" | "tool";

export type ToolCall = {
  id?: string;
  name: string;
  arguments: unknown;
};

export type ToolResult = {
  call_id?: string;
  name: string;
  content: unknown;
};

export type ApprovalRequestMessage = {
  id: string;
  summary: string;
  requestedCapabilities: string[];
  reason: string;
};

export type ApprovalDecisionMessage = {
  id: string;
  requestId: string;
  approved: boolean;
  reviewer: string;
  reason: string;
};

export type ApprovalSubject =
  | { type: "tool_call"; name: string; arguments: unknown }
  | { type: "runtime_command"; program: string; args: string[]; cwd: string }
  | { type: "patch_apply"; summary: string };

export type ApprovalRequest = {
  id: string;
  summary: string;
  subject: ApprovalSubject;
  requestedCapabilities: string[];
  reason: string;
};

export type ApprovalDecision = {
  id: string;
  requestId: string;
  approved: boolean;
  reviewer: string;
  reason: string;
};

export type PendingApproval = {
  request: ApprovalRequest;
  sessionId?: string;
  runId?: string;
  createdAtMs: number;
};

export type ResolvedApproval = PendingApproval & {
  decision: ApprovalDecision;
  resolvedAtMs: number;
  runtimeCommand?: RuntimeCommandResolution;
};

export type NetworkPolicy = "offline" | "approved_online";

export type RuntimeCommandSpec = {
  program: string;
  args: string[];
  cwd: string;
  timeoutMs: number;
  network: NetworkPolicy;
};

export type RuntimeCommandResult = {
  exitCode?: number;
  stdout: string;
  stderr: string;
  durationMs: number;
  timedOut: boolean;
};

export type RuntimeCommandResultMessage = {
  requestId: string;
  command: RuntimeCommandSpec;
  result: RuntimeCommandResult;
};

export type RuntimeCommandErrorMessage = {
  requestId: string;
  command: RuntimeCommandSpec;
  message: string;
};

export type RuntimeCommandResolution =
  | {
      status: "completed";
      messageId: string;
      requestId: string;
      command: RuntimeCommandSpec;
      result: RuntimeCommandResult;
    }
  | {
      status: "failed";
      messageId: string;
      requestId: string;
      command: RuntimeCommandSpec;
      message: string;
    };

export type ApprovalSubmission =
  | {
      status: "allowed";
      request: ApprovalRequest;
      runtimeCommand?: RuntimeCommandResolution;
    }
  | { status: "pending_manual_approval"; pending: PendingApproval };

export type MessagePart =
  | { type: "thinking"; text: string }
  | { type: "text"; text: string }
  | { type: "tool_call"; call: ToolCall }
  | { type: "tool_result"; result: ToolResult }
  | { type: "approval_request"; request: ApprovalRequestMessage }
  | { type: "approval_decision"; decision: ApprovalDecisionMessage }
  | { type: "runtime_command_result"; result: RuntimeCommandResultMessage }
  | { type: "runtime_command_error"; error: RuntimeCommandErrorMessage }
  | { type: "diff"; diff: unknown };

export type ConversationMessage = {
  id: string;
  role: MessageRole;
  parts: MessagePart[];
};

export type AgentTurnResponse = {
  messages: ConversationMessage[];
  done_reason?: string;
  tool_iteration_count: number;
};

export type AgentTurnTransport = "streaming" | "non_streaming_fallback";

export type SessionEvent =
  | {
      type: "agent_turn_completed";
      run_id: string;
      transport: AgentTurnTransport;
      base_url?: string;
      model?: string;
      messages: ConversationMessage[];
      done_reason?: string;
      tool_iteration_count: number;
    }
  | { type: "agent_turn_failed"; run_id: string; message: string }
  | { type: "agent_turn_cancelled"; run_id: string }
  | { type: "approval_requested"; run_id?: string; request: ApprovalRequest }
  | { type: "approval_resolved"; run_id?: string; decision: ApprovalDecision }
  | {
      type: "runtime_command_completed";
      message_id: string;
      run_id?: string;
      request_id: string;
      command: RuntimeCommandSpec;
      result: RuntimeCommandResult;
    }
  | {
      type: "runtime_command_failed";
      message_id: string;
      run_id?: string;
      request_id: string;
      command: RuntimeCommandSpec;
      message: string;
    };

export type SessionSnapshot = {
  id: string;
  title: string;
  workspaceRoot?: string;
  baseUrl?: string;
  model?: string;
  createdAtMs: number;
  updatedAtMs: number;
  messages: ConversationMessage[];
};

export type SessionSummary = {
  id: string;
  title: string;
  workspaceRoot?: string;
  model?: string;
  createdAtMs: number;
  updatedAtMs: number;
  messageCount: number;
};

export type WorkspaceSelection = {
  id: string;
  source_root: string;
};

export type ActiveRunContext = {
  runId: string;
  sessionId: string;
  workspaceId: string;
};

type ActiveSession = {
  sessionId: string;
  title: string;
  workspaceId: string;
  workspaceRoot: string;
  baseUrl?: string;
  model?: string;
};

export class SessionController {
  private activeSession: ActiveSession | null = null;
  private readonly history: ConversationMessage[] = [];

  get activeSessionId(): string | null {
    return this.activeSession?.sessionId ?? null;
  }

  get selectedWorkspaceId(): string | null {
    return this.activeSession?.workspaceId ?? null;
  }

  get selectedWorkspaceRoot(): string | null {
    return this.activeSession?.workspaceRoot ?? null;
  }

  get baseUrl(): string | null {
    return this.activeSession?.baseUrl ?? null;
  }

  get model(): string | null {
    return this.activeSession?.model ?? null;
  }

  get messages(): ConversationMessage[] {
    return this.history;
  }

  activateSession(session: SessionSnapshot, workspace: WorkspaceSelection) {
    this.activeSession = {
      sessionId: session.id,
      title: session.title,
      workspaceId: workspace.id,
      workspaceRoot: workspace.source_root,
      baseUrl: session.baseUrl,
      model: session.model,
    };
    this.replaceMessages(session.messages);
  }

  createRunContext(): ActiveRunContext | null {
    if (!this.activeSession) {
      return null;
    }

    return {
      runId: crypto.randomUUID(),
      sessionId: this.activeSession.sessionId,
      workspaceId: this.activeSession.workspaceId,
    };
  }

  replaceMessages(messages: ConversationMessage[]) {
    this.history.splice(0, this.history.length, ...messages);
  }

  findMessage(messageId: string): ConversationMessage | undefined {
    return this.history.find((message) => message.id === messageId);
  }

  appendMessage(message: ConversationMessage) {
    this.history.push(message);
  }

  appendTextPart(messageId: string, type: "thinking" | "text", text: string) {
    const message = this.findMessage(messageId);
    if (!message) return;

    const existing = message.parts.find((part) => part.type === type);
    if (existing?.type === type) {
      existing.text += text;
      return;
    }

    message.parts.push({ type, text });
  }

  appendToolCall(messageId: string, call: ToolCall) {
    const message = this.findMessage(messageId);
    if (!message) return;

    message.parts.push({ type: "tool_call", call });
  }

  removeMessagesById(messageIds: Set<string>): boolean {
    if (messageIds.size === 0) return false;

    const originalLength = this.history.length;
    for (let index = this.history.length - 1; index >= 0; index -= 1) {
      if (messageIds.has(this.history[index].id)) {
        this.history.splice(index, 1);
      }
    }

    return this.history.length !== originalLength;
  }

  removeEmptyAssistantMessages(messageIds: Set<string>): boolean {
    if (messageIds.size === 0) return false;

    const originalLength = this.history.length;
    for (let index = this.history.length - 1; index >= 0; index -= 1) {
      const message = this.history[index];
      if (
        message.role === "assistant" &&
        message.parts.length === 0 &&
        messageIds.has(message.id)
      ) {
        this.history.splice(index, 1);
      }
    }

    return this.history.length !== originalLength;
  }

  reconcileRunMessages(activeRunMessageIds: Set<string>, messages: ConversationMessage[]) {
    this.removeMessagesById(activeRunMessageIds);
    this.history.push(...messages);
  }
}

export async function createSessionForWorkspace(
  workspace: WorkspaceSelection,
  baseUrl?: string,
  model?: string,
): Promise<SessionSnapshot> {
  return invoke("create_session", {
    request: {
      title: workspaceTitle(workspace.source_root),
      workspaceRoot: workspace.source_root,
      baseUrl,
      model,
    },
  });
}

export async function appendSessionEvent(sessionId: string, event: SessionEvent) {
  await invoke("append_session_event", {
    sessionId,
    event,
  });
}

export async function tryAppendSessionEvent(
  sessionId: string,
  event: SessionEvent,
): Promise<string | null> {
  try {
    await appendSessionEvent(sessionId, event);
    return null;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

export async function listSessions(): Promise<SessionSummary[]> {
  return invoke("list_sessions");
}

export async function loadSession(sessionId: string): Promise<SessionSnapshot> {
  return invoke("load_session", { sessionId });
}

export async function selectWorkspacePath(sourceRoot: string): Promise<WorkspaceSelection> {
  return invoke("select_workspace_path", { sourceRoot });
}

export async function listPendingApprovals(): Promise<PendingApproval[]> {
  return invoke("list_pending_approvals");
}

export async function requestRuntimeCommandApproval(
  command: RuntimeCommandSpec,
  sessionId: string,
  workspaceId: string,
  runId?: string,
): Promise<ApprovalSubmission> {
  return invoke("request_runtime_command_approval", {
    request: {
      sessionId,
      runId,
      workspaceId,
      command,
    },
  });
}

export async function resolveApproval(
  requestId: string,
  approved: boolean,
  reason: string,
): Promise<ResolvedApproval> {
  return invoke("resolve_approval", { requestId, approved, reason });
}

export function workspaceTitle(path: string): string {
  return path.split(/[\\/]+/).filter(Boolean).pop() ?? "Workspace session";
}
