import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
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

type ToolProbeResponse = {
  first_thinking?: string;
  tool_call: ToolCall;
  tool_result: ToolResult;
  final_thinking?: string;
  final_content: string;
  done_reason?: string;
};

const app = document.querySelector<HTMLElement>("#app");

if (!app) {
  throw new Error("Missing #app root");
}

app.innerHTML = `
  <section class="shell">
    <header class="topbar">
      <div>
        <p class="eyebrow">Ollama Cowork</p>
        <h1>Local agent runtime</h1>
      </div>
      <div class="actions">
        <button id="probe">Probe Ollama</button>
        <button id="tool-probe">Run Tool Probe</button>
      </div>
    </header>

    <section class="panel">
      <div class="field-row">
        <label for="workspace-root">Workspace root</label>
        <div class="field-actions">
          <button id="choose-workspace" type="button">Choose folder</button>
        </div>
      </div>
      <input id="workspace-root" placeholder="Choose a workspace folder" spellcheck="false" />
      <label for="base-url">Ollama base URL</label>
      <input id="base-url" value="http://127.0.0.1:11434" spellcheck="false" />
      <label for="model">Model</label>
      <input id="model" value="gemma4:12b" spellcheck="false" />
      <p class="hint">Tool calls operate relative to the selected workspace. Model calls happen in the host app, not inside sandboxed commands.</p>
    </section>

    <section class="stack">
      <article class="message">
        <h2>Probe Result</h2>
        <pre id="output">Ready.</pre>
      </article>

      <article class="message">
        <button class="message-title" aria-expanded="false" aria-controls="thinking-body">
          Thinking
        </button>
        <pre id="thinking-body" hidden>Thinking blocks will render here as collapsible model reasoning.</pre>
      </article>

      <article class="message">
        <h2>Tool Call</h2>
        <pre id="tool-call">No tool call yet.</pre>
      </article>

      <article class="message">
        <h2>Tool Result</h2>
        <pre id="tool-result">No tool result yet.</pre>
      </article>

      <article class="message">
        <h2>Assistant Summary</h2>
        <pre id="final-content">No final answer yet.</pre>
      </article>
    </section>
  </section>
`;

const output = document.querySelector<HTMLPreElement>("#output");
const probe = document.querySelector<HTMLButtonElement>("#probe");
const toolProbe = document.querySelector<HTMLButtonElement>("#tool-probe");
const chooseWorkspace = document.querySelector<HTMLButtonElement>("#choose-workspace");
const workspaceRoot = document.querySelector<HTMLInputElement>("#workspace-root");
const baseUrl = document.querySelector<HTMLInputElement>("#base-url");
const model = document.querySelector<HTMLInputElement>("#model");
const thinkingButton = document.querySelector<HTMLButtonElement>(".message-title");
const thinkingBody = document.querySelector<HTMLPreElement>("#thinking-body");
const toolCall = document.querySelector<HTMLPreElement>("#tool-call");
const toolResult = document.querySelector<HTMLPreElement>("#tool-result");
const finalContent = document.querySelector<HTMLPreElement>("#final-content");

thinkingButton?.addEventListener("click", () => {
  if (!thinkingBody) return;
  const isHidden = thinkingBody.hidden;
  thinkingBody.hidden = !isHidden;
  thinkingButton.setAttribute("aria-expanded", String(isHidden));
});

function updateToolProbeState() {
  if (!toolProbe || !workspaceRoot) return;
  toolProbe.disabled = workspaceRoot.value.trim().length === 0;
}

workspaceRoot?.addEventListener("input", updateToolProbeState);
updateToolProbeState();

chooseWorkspace?.addEventListener("click", async () => {
  if (!workspaceRoot || !chooseWorkspace) return;

  chooseWorkspace.disabled = true;
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Choose workspace folder",
    });

    if (typeof selected === "string") {
      workspaceRoot.value = selected;
      updateToolProbeState();
    }
  } catch (error) {
    output?.replaceChildren(
      document.createTextNode(error instanceof Error ? error.message : String(error)),
    );
  } finally {
    chooseWorkspace.disabled = false;
  }
});

probe?.addEventListener("click", async () => {
  if (!output || !baseUrl) return;

  output.textContent = "Probing Ollama...";
  probe.disabled = true;

  try {
    const result = await invoke<ProbeOllamaResponse>("probe_ollama", {
      baseUrl: baseUrl.value,
    });
    output.textContent = JSON.stringify(result, null, 2);
  } catch (error) {
    output.textContent = error instanceof Error ? error.message : String(error);
  } finally {
    probe.disabled = false;
  }
});

toolProbe?.addEventListener("click", async () => {
  if (
    !output ||
    !baseUrl ||
    !model ||
    !workspaceRoot ||
    !thinkingBody ||
    !toolCall ||
    !toolResult ||
    !finalContent
  ) {
    return;
  }

  if (workspaceRoot.value.trim().length === 0) {
    output.textContent = "Choose a workspace folder before running the tool probe.";
    return;
  }

  output.textContent = "Running Gemma tool loop...";
  thinkingBody.textContent = "";
  toolCall.textContent = "Waiting for model tool call...";
  toolResult.textContent = "Waiting for Rust tool execution...";
  finalContent.textContent = "Waiting for final summary...";
  toolProbe.disabled = true;

  try {
    const result = await invoke<ToolProbeResponse>("run_tool_probe", {
      baseUrl: baseUrl.value,
      model: model.value,
      workspaceRoot: workspaceRoot.value,
    });

    output.textContent = JSON.stringify(
      {
        done_reason: result.done_reason,
      },
      null,
      2,
    );
    thinkingBody.textContent = [result.first_thinking, result.final_thinking]
      .filter(Boolean)
      .join("\n\n---\n\n");
    toolCall.textContent = JSON.stringify(result.tool_call, null, 2);
    toolResult.textContent = JSON.stringify(result.tool_result, null, 2);
    finalContent.textContent = result.final_content || "(No final content returned.)";
  } catch (error) {
    output.textContent = error instanceof Error ? error.message : String(error);
  } finally {
    updateToolProbeState();
  }
});
