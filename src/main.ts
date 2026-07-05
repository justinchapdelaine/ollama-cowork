import { invoke } from "@tauri-apps/api/core";
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
let isRunning = false;
let activeRunId: string | null = null;
const history: ConversationMessage[] = [];

function setStatus(value: string) {
  if (output) {
    output.textContent = value;
  }
}

function updateReadyState() {
  const hasWorkspace = selectedWorkspaceId !== null;
  if (toolProbe) {
    toolProbe.disabled = !hasWorkspace || isRunning;
  }
  if (promptInput) {
    promptInput.disabled = !hasWorkspace || isRunning;
  }
  if (send) {
    send.disabled = !hasWorkspace || isRunning;
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
    body.textContent = "(No renderable message parts.)";
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

chooseWorkspace?.addEventListener("click", async () => {
  if (!workspaceRoot || !chooseWorkspace) return;

  chooseWorkspace.disabled = true;
  try {
    const selected = await invoke<WorkspaceSelection | null>("choose_workspace");

    if (selected) {
      selectedWorkspaceId = selected.id;
      workspaceRoot.value = selected.source_root;
      setStatus("Workspace selected. Ready for a prompt.");
      updateReadyState();
    }
  } catch (error) {
    setStatus(error instanceof Error ? error.message : String(error));
  } finally {
    chooseWorkspace.disabled = false;
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

  if (!baseUrl || !model || !promptInput || !selectedWorkspaceId) {
    setStatus("Choose a workspace folder before sending a prompt.");
    return;
  }

  const userPrompt = promptInput.value.trim();
  if (!userPrompt) {
    return;
  }

  isRunning = true;
  activeRunId = crypto.randomUUID();
  updateReadyState();
  setStatus("Running agent turn...");
  promptInput.value = "";

  try {
    const result = await invoke<AgentTurnResponse>("run_agent_turn", {
      request: {
        baseUrl: baseUrl.value,
        model: model.value,
        workspaceId: selectedWorkspaceId,
        runId: activeRunId,
        userPrompt,
        history,
      },
    });

    history.push(...result.messages);
    renderConversation();
    setStatus(
      formatJson({
        done_reason: result.done_reason,
        tool_iteration_count: result.tool_iteration_count,
        appended_messages: result.messages.length,
      }),
    );
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setStatus(message.includes("agent run cancelled") ? "Run cancelled." : message);
  } finally {
    isRunning = false;
    activeRunId = null;
    updateReadyState();
    promptInput.focus();
  }
});

renderConversation();
updateReadyState();
