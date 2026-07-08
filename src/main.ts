import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  type AgentTurnResponse,
  type ConversationMessage,
  type MessageRole,
  type PendingApproval,
  type SessionEvent,
  type SessionSummary,
  type ToolCall,
  type ToolResult,
  type WorkspaceSelection,
  createSessionForWorkspace,
  listPendingApprovals,
  listSessions,
  loadSession,
  resolveApproval,
  selectWorkspacePath,
  SessionController,
  tryAppendSessionEvent,
} from "./session";
import "./styles.css";

type ProbeOllamaResponse = {
  base_url: string;
  version?: string;
  models: Array<{
    name: string;
    family?: string;
    parameter_size?: string;
    capabilities: string[];
  }>;
};

type AgentRunEvent =
  | { type: "message_appended"; message: ConversationMessage }
  | { type: "assistant_started"; message: ConversationMessage }
  | { type: "thinking_delta"; message_id: string; text: string }
  | { type: "content_delta"; message_id: string; text: string }
  | { type: "tool_call"; message_id: string; call: ToolCall }
  | {
      type: "completed";
      done_reason?: string;
      tool_iteration_count: number;
      appended_messages: number;
    }
  | { type: "cancelled" }
  | { type: "error"; message: string };

type AgentRunEventEnvelope = {
  runId: string;
  event: AgentRunEvent;
};

type ToolProbeResponse = {
  first_thinking?: string;
  tool_call: ToolCall;
  tool_result: ToolResult;
  final_thinking?: string;
  final_content: string;
  done_reason?: string;
};

const app = document.querySelector<HTMLElement>("#app");
const AGENT_RUN_EVENT = "agent-run-event";

if (!app) {
  throw new Error("Missing #app root");
}

app.innerHTML = `
  <main class="shell">
    <header class="topbar">
      <div>
        <p class="eyebrow">Ollama Cowork</p>
        <h1>Local agent runtime</h1>
      </div>
      <div class="actions">
        <button id="probe" type="button">Probe Ollama</button>
        <button id="tool-probe" type="button">Run Tool Probe</button>
      </div>
    </header>

    <section class="panel settings-panel" aria-label="Runtime settings">
      <div class="field-row">
        <label for="workspace-root">Workspace root</label>
        <div class="field-actions">
          <button id="choose-workspace" type="button">Choose folder</button>
        </div>
      </div>
      <input id="workspace-root" placeholder="Choose a workspace folder" spellcheck="false" readonly />
      <div class="settings-grid">
        <div>
          <label for="base-url">Ollama base URL</label>
          <input id="base-url" value="http://127.0.0.1:11434" spellcheck="false" />
        </div>
        <div>
          <label for="model">Model</label>
          <input id="model" value="gemma4:12b" spellcheck="false" />
        </div>
      </div>
      <p class="hint">Tool calls operate relative to the selected workspace. Model calls happen in the host app, not inside sandboxed commands.</p>
    </section>

    <section class="agent-layout">
      <section class="chat-panel" aria-label="Conversation">
        <div id="conversation" class="conversation" aria-live="polite">
          <article class="empty-state">
            <h2>Ask about the selected workspace</h2>
            <p>Start with a question like "Review the repository structure" or "Find where Ollama requests are handled."</p>
          </article>
        </div>

        <form id="composer" class="composer">
          <textarea id="prompt" rows="3" placeholder="Ask Ollama Cowork to inspect the workspace..." disabled></textarea>
          <div class="composer-actions">
            <button id="send" type="submit" disabled>Send</button>
            <button id="cancel-run" type="button" disabled>Cancel</button>
          </div>
        </form>
      </section>

      <aside class="diagnostics" aria-label="Diagnostics">
        <article class="message recent-sessions">
          <div class="panel-heading">
            <h2>Recent Sessions</h2>
            <button id="refresh-sessions" class="secondary" type="button">Refresh</button>
          </div>
          <div id="session-list" class="session-list">No saved sessions yet.</div>
        </article>

        <article class="message pending-approvals">
          <div class="panel-heading">
            <h2>Pending Approvals</h2>
            <button id="refresh-approvals" class="secondary" type="button">Refresh</button>
          </div>
          <div id="approval-list" class="approval-list">No pending approvals.</div>
        </article>

        <article class="message">
          <h2>Status</h2>
          <pre id="output">Choose a workspace folder to start.</pre>
        </article>

        <article class="message">
          <h2>Last Tool Probe</h2>
          <pre id="tool-probe-output">No tool probe yet.</pre>
        </article>
      </aside>
    </section>
  </main>
`;

const output = document.querySelector<HTMLPreElement>("#output");
const probe = document.querySelector<HTMLButtonElement>("#probe");
const toolProbe = document.querySelector<HTMLButtonElement>("#tool-probe");
const chooseWorkspace = document.querySelector<HTMLButtonElement>("#choose-workspace");
const workspaceRoot = document.querySelector<HTMLInputElement>("#workspace-root");
const baseUrl = document.querySelector<HTMLInputElement>("#base-url");
const model = document.querySelector<HTMLInputElement>("#model");
const conversation = document.querySelector<HTMLElement>("#conversation");
const composer = document.querySelector<HTMLFormElement>("#composer");
const promptInput = document.querySelector<HTMLTextAreaElement>("#prompt");
const send = document.querySelector<HTMLButtonElement>("#send");
const cancelRun = document.querySelector<HTMLButtonElement>("#cancel-run");
const toolProbeOutput = document.querySelector<HTMLPreElement>("#tool-probe-output");
const refreshSessions = document.querySelector<HTMLButtonElement>("#refresh-sessions");
const sessionList = document.querySelector<HTMLElement>("#session-list");
const refreshApprovals = document.querySelector<HTMLButtonElement>("#refresh-approvals");
const approvalList = document.querySelector<HTMLElement>("#approval-list");

const sessions = new SessionController();
let isRunning = false;
let activeRunId: string | null = null;
let activeRunHadModelEvents = false;
const activeRunMessageIds = new Set<string>();
const activeRunAssistantMessageIds = new Set<string>();

function setStatus(value: string) {
  if (output) {
    output.textContent = value;
  }
}

function updateReadyState() {
  const hasWorkspace = sessions.selectedWorkspaceId !== null;
  const hasSession = sessions.activeSessionId !== null;
  if (chooseWorkspace) {
    chooseWorkspace.disabled = isRunning;
  }
  if (toolProbe) {
    toolProbe.disabled = !hasWorkspace || isRunning;
  }
  if (promptInput) {
    promptInput.disabled = !hasWorkspace || !hasSession || isRunning;
  }
  if (send) {
    send.disabled = !hasWorkspace || !hasSession || isRunning;
  }
  if (cancelRun) {
    cancelRun.disabled = !isRunning || activeRunId === null;
  }
  if (refreshSessions) {
    refreshSessions.disabled = isRunning;
  }
  for (const button of sessionList?.querySelectorAll<HTMLButtonElement>("button") ?? []) {
    button.disabled = isRunning;
  }
}

function roleLabel(role: MessageRole): string {
  if (role === "tool") return "Tool";
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function formatJson(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function formatSessionTime(value: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(value));
}

function shortWorkspaceName(path?: string): string {
  if (!path) return "No workspace";
  return path.split(/[\\/]+/).filter(Boolean).pop() ?? path;
}

function renderSessionList(summaries: SessionSummary[]) {
  if (!sessionList) return;

  sessionList.replaceChildren();
  if (summaries.length === 0) {
    const empty = document.createElement("p");
    empty.className = "session-empty";
    empty.textContent = "No saved sessions yet.";
    sessionList.append(empty);
    return;
  }

  for (const summary of summaries) {
    const button = document.createElement("button");
    button.className = "session-item";
    button.type = "button";
    button.dataset.sessionId = summary.id;

    const title = document.createElement("span");
    title.className = "session-title";
    title.textContent = summary.title || shortWorkspaceName(summary.workspaceRoot);

    const meta = document.createElement("span");
    meta.className = "session-meta";
    meta.textContent = `${shortWorkspaceName(summary.workspaceRoot)} - ${summary.messageCount} messages - ${formatSessionTime(summary.updatedAtMs)}`;

    button.append(title, meta);
    sessionList.append(button);
  }

  updateReadyState();
}

async function refreshSessionList() {
  if (!sessionList) return;

  sessionList.textContent = "Loading sessions...";
  try {
    renderSessionList(await listSessions());
  } catch (error) {
    sessionList.textContent = `Could not load sessions: ${
      error instanceof Error ? error.message : String(error)
    }`;
  } finally {
    updateReadyState();
  }
}

function renderApprovalList(approvals: PendingApproval[]) {
  if (!approvalList) return;

  approvalList.replaceChildren();
  if (approvals.length === 0) {
    const empty = document.createElement("p");
    empty.className = "session-empty";
    empty.textContent = "No pending approvals.";
    approvalList.append(empty);
    return;
  }

  for (const approval of approvals) {
    const item = document.createElement("article");
    item.className = "approval-item";
    item.dataset.requestId = approval.request.id;

    const title = document.createElement("div");
    title.className = "approval-title";
    title.textContent = approval.request.summary;

    const meta = document.createElement("div");
    meta.className = "approval-meta";
    meta.textContent = `${approval.request.requestedCapabilities.join(", ")} - ${formatSessionTime(approval.createdAtMs)}`;

    const reason = document.createElement("p");
    reason.className = "approval-reason";
    reason.textContent = approval.request.reason;

    const actions = document.createElement("div");
    actions.className = "approval-actions";

    const approve = document.createElement("button");
    approve.type = "button";
    approve.dataset.approvalAction = "approve";
    approve.textContent = "Approve";

    const deny = document.createElement("button");
    deny.type = "button";
    deny.className = "secondary";
    deny.dataset.approvalAction = "deny";
    deny.textContent = "Deny";

    actions.append(approve, deny);
    item.append(title, meta, reason, actions);
    approvalList.append(item);
  }
}

async function refreshApprovalList() {
  if (!approvalList) return;

  approvalList.textContent = "Loading approvals...";
  try {
    renderApprovalList(await listPendingApprovals());
  } catch (error) {
    approvalList.textContent = `Could not load approvals: ${
      error instanceof Error ? error.message : String(error)
    }`;
  }
}

async function resolvePendingApproval(requestId: string, approved: boolean) {
  setStatus(approved ? "Approving request..." : "Denying request...");
  try {
    await resolveApproval(
      requestId,
      approved,
      approved ? "Approved by user." : "Denied by user.",
    );
    setStatus(approved ? "Approval granted." : "Approval denied.");
    await refreshApprovalList();
    await refreshSessionList();
  } catch (error) {
    setStatus(error instanceof Error ? error.message : String(error));
  }
}

function syncActiveSessionFields() {
  if (workspaceRoot && sessions.selectedWorkspaceRoot) {
    workspaceRoot.value = sessions.selectedWorkspaceRoot;
  }
  if (baseUrl && sessions.baseUrl) {
    baseUrl.value = sessions.baseUrl;
  }
  if (model && sessions.model) {
    model.value = sessions.model;
  }
}

async function loadExistingSession(sessionId: string) {
  if (isRunning) {
    setStatus("Wait for the active run to finish before loading a session.");
    updateReadyState();
    return;
  }

  setStatus("Loading session...");
  try {
    const session = await loadSession(sessionId);
    if (!session.workspaceRoot) {
      throw new Error("saved session does not include a workspace root");
    }
    const workspace = await selectWorkspacePath(session.workspaceRoot);

    sessions.activateSession(session, workspace);
    syncActiveSessionFields();
    renderConversation();
    setStatus(`Loaded session: ${session.title}`);
  } catch (error) {
    setStatus(error instanceof Error ? error.message : String(error));
  } finally {
    await refreshSessionList();
    updateReadyState();
  }
}

function renderConversation() {
  if (!conversation) return;

  conversation.replaceChildren();
  const visibleMessages = sessions.messages.filter((message) => message.role !== "system");

  if (visibleMessages.length === 0) {
    const empty = document.createElement("article");
    empty.className = "empty-state";
    empty.innerHTML = `
      <h2>Ask about the selected workspace</h2>
      <p>Start with a question like "Review the repository structure" or "Find where Ollama requests are handled."</p>
    `;
    conversation.append(empty);
    return;
  }

  for (const message of visibleMessages) {
    conversation.append(renderMessage(message));
  }

  conversation.scrollTop = conversation.scrollHeight;
}

function renderMessage(message: ConversationMessage): HTMLElement {
  const article = document.createElement("article");
  article.className = `chat-message ${message.role}`;

  const header = document.createElement("div");
  header.className = "chat-message-header";
  header.textContent = roleLabel(message.role);
  article.append(header);

  for (const part of message.parts) {
    if (part.type === "text") {
      const body = document.createElement("p");
      body.className = "chat-text";
      body.textContent = part.text || "(No text returned.)";
      article.append(body);
      continue;
    }

    if (part.type === "thinking") {
      const details = document.createElement("details");
      details.className = "thinking-block";
      const summary = document.createElement("summary");
      summary.textContent = "Thinking";
      const pre = document.createElement("pre");
      pre.textContent = part.text;
      details.append(summary, pre);
      article.append(details);
      continue;
    }

    if (part.type === "tool_call") {
      article.append(renderToolBlock("Tool Call", part.call));
      continue;
    }

    if (part.type === "tool_result") {
      article.append(renderToolBlock(`Tool Result: ${part.result.name}`, part.result.content));
      continue;
    }

    if (part.type === "approval_request") {
      article.append(renderToolBlock("Approval Request", part.request));
      continue;
    }

    if (part.type === "approval_decision") {
      article.append(
        renderToolBlock(
          part.decision.approved ? "Approval Granted" : "Approval Denied",
          part.decision,
        ),
      );
    }
  }

  if (article.childElementCount === 1) {
    const body = document.createElement("p");
    body.className = "chat-text muted";
    body.textContent =
      message.role === "assistant" && isRunning
        ? "Waiting for streamed response..."
        : "(No renderable message parts.)";
    article.append(body);
  }

  return article;
}

function renderToolBlock(title: string, value: unknown): HTMLElement {
  const details = document.createElement("details");
  details.className = "tool-block";
  details.open = true;
  const summary = document.createElement("summary");
  summary.textContent = title;
  const pre = document.createElement("pre");
  pre.textContent = formatJson(value);
  details.append(summary, pre);
  return details;
}

function trackActiveRunMessage(message: ConversationMessage) {
  activeRunMessageIds.add(message.id);
  if (message.role === "assistant") {
    activeRunAssistantMessageIds.add(message.id);
  }
}

function removeActiveRunMessages() {
  return sessions.removeMessagesById(activeRunMessageIds);
}

function reconcileActiveRunMessages(messages: ConversationMessage[]) {
  sessions.reconcileRunMessages(activeRunMessageIds, messages);
  activeRunMessageIds.clear();
  activeRunAssistantMessageIds.clear();
  renderConversation();
}

async function appendSessionEventOrStatus(sessionId: string, event: SessionEvent) {
  const message = await tryAppendSessionEvent(sessionId, event);
  if (message) {
    setStatus(`Session save failed: ${message}`);
  }
}

function removeEmptyActiveAssistantMessages() {
  if (sessions.removeEmptyAssistantMessages(activeRunAssistantMessageIds)) {
    renderConversation();
  }
}

function applyAgentRunEvent(envelope: AgentRunEventEnvelope) {
  if (envelope.runId !== activeRunId) return;

  const { event } = envelope;

  if (event.type === "assistant_started") {
    trackActiveRunMessage(event.message);
    sessions.appendMessage(event.message);
    renderConversation();
    return;
  }

  if (event.type === "message_appended") {
    trackActiveRunMessage(event.message);
    sessions.appendMessage(event.message);
    renderConversation();
    return;
  }

  if (event.type === "thinking_delta") {
    activeRunHadModelEvents = true;
    sessions.appendTextPart(event.message_id, "thinking", event.text);
    renderConversation();
    return;
  }

  if (event.type === "content_delta") {
    activeRunHadModelEvents = true;
    sessions.appendTextPart(event.message_id, "text", event.text);
    renderConversation();
    return;
  }

  if (event.type === "tool_call") {
    activeRunHadModelEvents = true;
    sessions.appendToolCall(event.message_id, event.call);
    renderConversation();
    return;
  }

  if (event.type === "completed") {
    setStatus(
      formatJson({
        done_reason: event.done_reason,
        tool_iteration_count: event.tool_iteration_count,
        appended_messages: event.appended_messages,
      }),
    );
    return;
  }

  if (event.type === "cancelled") {
    removeEmptyActiveAssistantMessages();
    activeRunAssistantMessageIds.clear();
    setStatus("Run cancelled.");
    return;
  }

  if (event.type === "error") {
    removeEmptyActiveAssistantMessages();
    activeRunAssistantMessageIds.clear();
    setStatus(event.message);
  }
}

void listen<AgentRunEventEnvelope>(AGENT_RUN_EVENT, (event) => {
  applyAgentRunEvent(event.payload);
});

refreshSessions?.addEventListener("click", () => {
  void refreshSessionList();
});

refreshApprovals?.addEventListener("click", () => {
  void refreshApprovalList();
});

approvalList?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>(
    "button[data-approval-action]",
  );
  const item = button?.closest<HTMLElement>("[data-request-id]");
  const requestId = item?.dataset.requestId;
  if (!button?.dataset.approvalAction || !requestId) return;

  void resolvePendingApproval(requestId, button.dataset.approvalAction === "approve");
});

sessionList?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("button[data-session-id]");
  if (!button?.dataset.sessionId) return;

  void loadExistingSession(button.dataset.sessionId);
});

chooseWorkspace?.addEventListener("click", async () => {
  if (!workspaceRoot || !chooseWorkspace) return;
  if (isRunning) {
    setStatus("Wait for the active run to finish before switching workspaces.");
    updateReadyState();
    return;
  }

  chooseWorkspace.disabled = true;
  try {
    const selected = await invoke<WorkspaceSelection | null>("choose_workspace");

    if (selected) {
      const session = await createSessionForWorkspace(selected, baseUrl?.value, model?.value);

      sessions.activateSession(session, selected);
      syncActiveSessionFields();
      renderConversation();
      setStatus("Workspace selected. Session started.");
      await refreshSessionList();
      updateReadyState();
    }
  } catch (error) {
    setStatus(error instanceof Error ? error.message : String(error));
  } finally {
    updateReadyState();
  }
});

probe?.addEventListener("click", async () => {
  if (!baseUrl || !probe) return;

  setStatus("Probing Ollama...");
  probe.disabled = true;

  try {
    const result = await invoke<ProbeOllamaResponse>("probe_ollama", {
      baseUrl: baseUrl.value,
    });
    setStatus(formatJson(result));
  } catch (error) {
    setStatus(error instanceof Error ? error.message : String(error));
  } finally {
    probe.disabled = false;
  }
});

toolProbe?.addEventListener("click", async () => {
  const workspaceId = sessions.selectedWorkspaceId;
  if (!baseUrl || !model || !toolProbeOutput || !workspaceId) {
    setStatus("Choose a workspace folder before running the tool probe.");
    return;
  }

  setStatus("Running diagnostic tool probe...");
  toolProbeOutput.textContent = "Waiting for model tool call...";
  toolProbe.disabled = true;

  try {
    const result = await invoke<ToolProbeResponse>("run_tool_probe", {
      baseUrl: baseUrl.value,
      model: model.value,
      workspaceId,
    });

    toolProbeOutput.textContent = formatJson({
      done_reason: result.done_reason,
      first_thinking: result.first_thinking,
      tool_call: result.tool_call,
      tool_result: result.tool_result,
      final_thinking: result.final_thinking,
      final_content: result.final_content,
    });
    setStatus("Tool probe completed.");
  } catch (error) {
    setStatus(error instanceof Error ? error.message : String(error));
  } finally {
    updateReadyState();
  }
});

cancelRun?.addEventListener("click", async () => {
  if (!activeRunId) return;

  cancelRun.disabled = true;
  setStatus("Cancelling active agent run...");

  try {
    const cancelled = await invoke<boolean>("cancel_agent_run", {
      runId: activeRunId,
    });
    setStatus(cancelled ? "Cancel requested." : "No active run found to cancel.");
  } catch (error) {
    setStatus(error instanceof Error ? error.message : String(error));
  } finally {
    updateReadyState();
  }
});

composer?.addEventListener("submit", async (event) => {
  event.preventDefault();

  if (!baseUrl || !model || !promptInput) {
    setStatus("Choose a workspace folder before sending a prompt.");
    return;
  }

  const runContext = sessions.createRunContext();
  if (!runContext) {
    setStatus("Choose a workspace folder before sending a prompt.");
    updateReadyState();
    return;
  }

  const userPrompt = promptInput.value.trim();
  if (!userPrompt) {
    return;
  }

  isRunning = true;
  activeRunId = runContext.runId;
  activeRunHadModelEvents = false;
  updateReadyState();
  setStatus("Running agent turn...");
  promptInput.value = "";

  try {
    const result = await invoke<AgentTurnResponse>("run_agent_turn_stream", {
      request: {
        baseUrl: baseUrl.value,
        model: model.value,
        workspaceId: runContext.workspaceId,
        runId: runContext.runId,
        userPrompt,
        history: sessions.messages,
      },
    });

    reconcileActiveRunMessages(result.messages);
    const sessionError = await tryAppendSessionEvent(runContext.sessionId, {
      type: "agent_turn_completed",
      run_id: runContext.runId,
      transport: "streaming",
      base_url: baseUrl.value,
      model: model.value,
      messages: result.messages,
      done_reason: result.done_reason,
      tool_iteration_count: result.tool_iteration_count,
    });
    setStatus(
      formatJson({
        done_reason: result.done_reason,
        tool_iteration_count: result.tool_iteration_count,
        appended_messages: result.messages.length,
        session_saved: sessionError === null,
        ...(sessionError ? { session_error: sessionError } : {}),
      }),
    );
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (!activeRunHadModelEvents && !message.includes("agent run cancelled")) {
      try {
        const fallbackHistory = sessions.messages.filter(
          (message) => !activeRunMessageIds.has(message.id),
        );
        const result = await invoke<AgentTurnResponse>("run_agent_turn", {
          request: {
            baseUrl: baseUrl.value,
            model: model.value,
            workspaceId: runContext.workspaceId,
            runId: runContext.runId,
            userPrompt,
            history: fallbackHistory,
          },
        });

        reconcileActiveRunMessages(result.messages);
        const sessionError = await tryAppendSessionEvent(runContext.sessionId, {
          type: "agent_turn_completed",
          run_id: runContext.runId,
          transport: "non_streaming_fallback",
          base_url: baseUrl.value,
          model: model.value,
          messages: result.messages,
          done_reason: result.done_reason,
          tool_iteration_count: result.tool_iteration_count,
        });
        setStatus(
          formatJson({
            done_reason: result.done_reason,
            tool_iteration_count: result.tool_iteration_count,
            appended_messages: result.messages.length,
            fallback: "non_streaming",
            session_saved: sessionError === null,
            ...(sessionError ? { session_error: sessionError } : {}),
          }),
        );
      } catch (fallbackError) {
        const fallbackMessage =
          fallbackError instanceof Error ? fallbackError.message : String(fallbackError);
        setStatus(fallbackMessage);
        await appendSessionEventOrStatus(runContext.sessionId, {
          type: "agent_turn_failed",
          run_id: runContext.runId,
          message: fallbackMessage,
        });
      }
    } else {
      removeEmptyActiveAssistantMessages();
      setStatus(message.includes("agent run cancelled") ? "Run cancelled." : message);
      await appendSessionEventOrStatus(
        runContext.sessionId,
        message.includes("agent run cancelled")
          ? { type: "agent_turn_cancelled", run_id: runContext.runId }
          : { type: "agent_turn_failed", run_id: runContext.runId, message },
      );
    }
  } finally {
    isRunning = false;
    activeRunId = null;
    activeRunHadModelEvents = false;
    activeRunMessageIds.clear();
    activeRunAssistantMessageIds.clear();
    updateReadyState();
    void refreshSessionList();
    promptInput.focus();
  }
});

renderConversation();
updateReadyState();
void refreshSessionList();
void refreshApprovalList();
