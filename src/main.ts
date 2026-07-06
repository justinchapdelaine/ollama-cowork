import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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

type MessageRole = "system" | "user" | "assistant" | "tool";

type ToolCall = {
  id?: string;
  name: string;
  arguments: unknown;
};

type ToolResult = {
  call_id?: string;
  name: string;
  content: unknown;
};

type MessagePart =
  | { type: "thinking"; text: string }
  | { type: "text"; text: string }
  | { type: "tool_call"; call: ToolCall }
  | { type: "tool_result"; result: ToolResult }
  | { type: "approval_request"; request: unknown }
  | { type: "diff"; diff: unknown };

type ConversationMessage = {
  id: string;
  role: MessageRole;
  parts: MessagePart[];
};

type AgentTurnResponse = {
  messages: ConversationMessage[];
  done_reason?: string;
  tool_iteration_count: number;
};

type AgentTurnTransport = "streaming" | "non_streaming_fallback";

type SessionEvent =
  | {
      type: "agent_turn_completed";
      run_id: string;
      transport: AgentTurnTransport;
      messages: ConversationMessage[];
      done_reason?: string;
      tool_iteration_count: number;
    }
  | { type: "agent_turn_failed"; run_id: string; message: string }
  | { type: "agent_turn_cancelled"; run_id: string };

type SessionSnapshot = {
  id: string;
  title: string;
  messages: ConversationMessage[];
};

type ActiveRunContext = {
  runId: string;
  sessionId: string;
  workspaceId: string;
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

type WorkspaceSelection = {
  id: string;
  source_root: string;
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

let selectedWorkspaceId: string | null = null;
let activeSessionId: string | null = null;
let isRunning = false;
let activeRunId: string | null = null;
let activeRunHadModelEvents = false;
const activeRunMessageIds = new Set<string>();
const activeRunAssistantMessageIds = new Set<string>();
const history: ConversationMessage[] = [];

function setStatus(value: string) {
  if (output) {
    output.textContent = value;
  }
}

function updateReadyState() {
  const hasWorkspace = selectedWorkspaceId !== null;
  const hasSession = activeSessionId !== null;
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
}

function roleLabel(role: MessageRole): string {
  if (role === "tool") return "Tool";
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function formatJson(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function workspaceTitle(path: string): string {
  return path.split(/[\\/]+/).filter(Boolean).pop() ?? "Workspace session";
}

function createActiveRunContext(): ActiveRunContext | null {
  if (!selectedWorkspaceId || !activeSessionId) {
    return null;
  }

  return {
    runId: crypto.randomUUID(),
    sessionId: activeSessionId,
    workspaceId: selectedWorkspaceId,
  };
}

function renderConversation() {
  if (!conversation) return;

  conversation.replaceChildren();
  const visibleMessages = history.filter((message) => message.role !== "system");

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

function findMessage(messageId: string): ConversationMessage | undefined {
  return history.find((message) => message.id === messageId);
}

function appendTextPart(
  messageId: string,
  type: "thinking" | "text",
  text: string,
) {
  const message = findMessage(messageId);
  if (!message) return;

  const existing = message.parts.find((part) => part.type === type);
  if (existing?.type === type) {
    existing.text += text;
    return;
  }

  message.parts.push({ type, text });
}

function appendToolCall(messageId: string, call: ToolCall) {
  const message = findMessage(messageId);
  if (!message) return;

  message.parts.push({ type: "tool_call", call });
}

function trackActiveRunMessage(message: ConversationMessage) {
  activeRunMessageIds.add(message.id);
  if (message.role === "assistant") {
    activeRunAssistantMessageIds.add(message.id);
  }
}

function removeActiveRunMessages() {
  if (activeRunMessageIds.size === 0) return false;

  const originalLength = history.length;
  for (let index = history.length - 1; index >= 0; index -= 1) {
    if (activeRunMessageIds.has(history[index].id)) {
      history.splice(index, 1);
    }
  }

  return history.length !== originalLength;
}

function reconcileActiveRunMessages(messages: ConversationMessage[]) {
  removeActiveRunMessages();
  history.push(...messages);
  activeRunMessageIds.clear();
  activeRunAssistantMessageIds.clear();
  renderConversation();
}

async function appendSessionEvent(sessionId: string, event: SessionEvent) {
  await invoke("append_session_event", {
    sessionId,
    event,
  });
}

async function tryAppendSessionEvent(
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

async function appendSessionEventOrStatus(sessionId: string, event: SessionEvent) {
  const message = await tryAppendSessionEvent(sessionId, event);
  if (message) {
    setStatus(`Session save failed: ${message}`);
  }
}

function removeEmptyActiveAssistantMessages() {
  if (activeRunAssistantMessageIds.size === 0) return;

  const originalLength = history.length;
  for (let index = history.length - 1; index >= 0; index -= 1) {
    const message = history[index];
    if (
      message.role === "assistant" &&
      message.parts.length === 0 &&
      activeRunAssistantMessageIds.has(message.id)
    ) {
      history.splice(index, 1);
    }
  }

  if (history.length !== originalLength) {
    renderConversation();
  }
}

function applyAgentRunEvent(envelope: AgentRunEventEnvelope) {
  if (envelope.runId !== activeRunId) return;

  const { event } = envelope;

  if (event.type === "assistant_started") {
    trackActiveRunMessage(event.message);
    history.push(event.message);
    renderConversation();
    return;
  }

  if (event.type === "message_appended") {
    trackActiveRunMessage(event.message);
    history.push(event.message);
    renderConversation();
    return;
  }

  if (event.type === "thinking_delta") {
    activeRunHadModelEvents = true;
    appendTextPart(event.message_id, "thinking", event.text);
    renderConversation();
    return;
  }

  if (event.type === "content_delta") {
    activeRunHadModelEvents = true;
    appendTextPart(event.message_id, "text", event.text);
    renderConversation();
    return;
  }

  if (event.type === "tool_call") {
    activeRunHadModelEvents = true;
    appendToolCall(event.message_id, event.call);
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
      const session = await invoke<SessionSnapshot>("create_session", {
        request: {
          title: workspaceTitle(selected.source_root),
          workspaceRoot: selected.source_root,
          baseUrl: baseUrl?.value,
          model: model?.value,
        },
      });

      selectedWorkspaceId = selected.id;
      activeSessionId = session.id;
      workspaceRoot.value = selected.source_root;
      history.splice(0, history.length, ...session.messages);
      renderConversation();
      setStatus("Workspace selected. Session started.");
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
  if (!baseUrl || !model || !toolProbeOutput || !selectedWorkspaceId) {
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
      workspaceId: selectedWorkspaceId,
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

  const runContext = createActiveRunContext();
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
        history,
      },
    });

    reconcileActiveRunMessages(result.messages);
    const sessionError = await tryAppendSessionEvent(runContext.sessionId, {
      type: "agent_turn_completed",
      run_id: runContext.runId,
      transport: "streaming",
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
        const fallbackHistory = history.filter(
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
    promptInput.focus();
  }
});

renderConversation();
updateReadyState();
