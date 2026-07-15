import type { ComponentHealth } from "./contracts"
import { isTerminalPhase, type WorkflowPhase, type WorkflowUiState } from "./workflow-state"

export interface WorkflowViewActions {
  selectDocument(): void
  setInstruction(value: string): void
  startWorkflow(): void
  approveOnce(): void
  reject(): void
  cancel(): void
  reset(): void
}

const phaseLabels: Record<WorkflowPhase, string> = {
  checking: "Checking prerequisites",
  blocked: "Setup required",
  ready: "Ready for a document",
  starting: "Starting isolated workflow",
  running: "Working",
  awaiting_approval: "Waiting for your approval",
  completed: "Revised copy created",
  rejected: "Change rejected",
  cancelled: "Workflow cancelled",
  failed: "Workflow failed",
}

const element = <K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] => {
  const value = document.createElement(tag)
  if (className) value.className = className
  if (text !== undefined) value.textContent = text
  return value
}

const button = (label: string, className: string, action: () => void): HTMLButtonElement => {
  const value = element("button", className, label)
  value.type = "button"
  value.addEventListener("click", action)
  return value
}

const componentCard = (name: string, value: ComponentHealth): HTMLElement => {
  const card = element("article", `component component--${value.state}`)
  const header = element("div", "component__header")
  header.append(element("h3", undefined, name), element("span", undefined, value.state))
  card.append(header, element("p", undefined, value.detail))
  if (value.version) card.append(element("code", undefined, value.version))
  return card
}

const healthPanel = (state: WorkflowUiState): HTMLElement => {
  const details = element("details", "health")
  const summary = element("summary")
  summary.append(
    element("span", "health__title", "Runtime prerequisites"),
    element(
      "span",
      `health__badge health__badge--${state.health?.ready ? "ready" : "blocked"}`,
      state.health?.ready ? "Ready" : "Attention needed",
    ),
  )
  details.append(summary)
  if (state.health) {
    const grid = element("section", "grid")
    grid.setAttribute("aria-label", "Backend health")
    grid.append(
      componentCard("opencode", state.health.opencode),
      componentCard("Sandbox Runtime", state.health.sandbox),
      componentCard("Runtime tools", state.health.runtimeTools),
      componentCard("Ollama endpoint", state.health.modelEndpoint),
    )
    details.append(grid)
  }
  return details
}

const artifactName = (path: string): string => path.split(/[\\/]/).at(-1) ?? "Revised DOCX"

const workflowPanel = (state: WorkflowUiState, actions: WorkflowViewActions): HTMLElement => {
  const panel = element("section", "workflow")
  panel.setAttribute("aria-labelledby", "workflow-title")
  const heading = element("div", "section-heading")
  const titleGroup = element("div")
  const title = element("h2", undefined, "Revise one Word document")
  title.id = "workflow-title"
  titleGroup.append(
    title,
    element(
      "p",
      undefined,
      "The original remains unchanged. Any approved edit is written to a new DOCX copy.",
    ),
  )
  const status = element("span", `phase phase--${state.phase}`, phaseLabels[state.phase])
  status.setAttribute("aria-live", "polite")
  heading.append(titleGroup, status)
  panel.append(heading)

  if (state.phase === "blocked" || state.phase === "checking") {
    panel.append(
      element(
        "p",
        "empty-state",
        state.phase === "checking"
          ? "Checking the trusted runtime before enabling document selection."
          : "Resolve the prerequisite shown above before starting a document workflow.",
      ),
    )
    return panel
  }

  const form = element("div", "workflow-form")
  const selectionRow = element("div", "selection-row")
  const select = button(
    state.selection ? "Choose a different DOCX" : "Choose DOCX",
    "button button--secondary",
    actions.selectDocument,
  )
  select.disabled = state.commandPending || state.phase !== "ready"
  selectionRow.append(
    select,
    element(
      "span",
      state.selection ? "selection selection--chosen" : "selection",
      state.selection?.displayName ?? "No document selected",
    ),
  )

  const label = element("label", "field-label", "Instruction")
  label.htmlFor = "workflow-instruction"
  const instruction = element("textarea", "instruction")
  instruction.id = "workflow-instruction"
  instruction.rows = 5
  instruction.maxLength = 16_384
  instruction.placeholder = "For example: Rewrite the Summary section to be clearer and more concise."
  instruction.value = state.instruction
  instruction.disabled = state.phase !== "ready" || state.commandPending

  const controls = element("div", "controls")
  const start = button("Start secure revision", "button button--primary", actions.startWorkflow)
  start.disabled =
    state.phase !== "ready" ||
    state.commandPending ||
    !state.selection ||
    state.instruction.trim().length === 0
  instruction.addEventListener("input", () => {
    actions.setInstruction(instruction.value)
    start.disabled = !state.selection || instruction.value.trim().length === 0
  })
  controls.append(start)
  form.append(selectionRow, label, instruction, controls)
  panel.append(form)

  if (state.assistantText.length > 0) {
    const transcript = element("section", "transcript")
    transcript.append(element("h3", undefined, "Assistant"))
    const content = element("div", "transcript__content")
    for (const message of state.assistantText) {
      content.append(element("p", undefined, message.text))
    }
    transcript.append(content)
    panel.append(transcript)
  }

  if (state.pendingAction) {
    const approval = element("section", "approval")
    approval.setAttribute("aria-live", "assertive")
    approval.append(
      element("p", "eyebrow", "APPROVAL REQUIRED"),
      element("h3", undefined, state.pendingAction.title),
      element("p", "approval__summary", state.pendingAction.summary),
    )
    const proposal = state.pendingAction.proposal
    const comparison = element("div", "proposal")
    comparison.append(element("h4", undefined, proposal.heading))
    const columns = element("div", "proposal__columns")
    const before = element("section", "proposal__column")
    before.append(element("h5", undefined, "Current text"))
    for (const paragraph of proposal.currentParagraphs) {
      before.append(element("p", undefined, paragraph))
    }
    const after = element("section", "proposal__column proposal__column--after")
    after.append(element("h5", undefined, "Proposed text"))
    for (const paragraph of proposal.replacementParagraphs) {
      after.append(element("p", undefined, paragraph))
    }
    columns.append(before, after)
    comparison.append(columns)
    approval.append(comparison)
    const decisions = element("div", "controls")
    const approve = button("Allow once", "button button--primary", actions.approveOnce)
    const reject = button("Reject", "button button--danger", actions.reject)
    const cancel = button("Cancel workflow", "button button--ghost", actions.cancel)
    approve.disabled = state.commandPending
    reject.disabled = state.commandPending
    cancel.disabled = state.commandPending
    decisions.append(approve, reject, cancel)
    approval.append(decisions)
    panel.append(approval)
  }

  if (state.artifact) {
    const artifact = element("section", "artifact")
    artifact.append(
      element("p", "eyebrow", "REVISED COPY"),
      element("h3", undefined, artifactName(state.artifact.path)),
      element("p", undefined, "Validated DOCX · original preserved"),
      element("code", undefined, `SHA-256 ${state.artifact.sha256}`),
    )
    panel.append(artifact)
  }

  if (state.error) {
    const error = element("section", "error")
    error.setAttribute("role", "alert")
    error.append(
      element("strong", undefined, state.error.message),
      element("code", undefined, state.error.code),
    )
    panel.append(error)
  }

  const active = state.phase === "starting" || state.phase === "running" || state.phase === "awaiting_approval"
  if (active && !state.pendingAction) {
    const activeControls = element("div", "controls controls--end")
    const cancel = button("Cancel workflow", "button button--ghost", actions.cancel)
    cancel.disabled = state.commandPending || state.jobId === null
    activeControls.append(cancel)
    panel.append(activeControls)
  }

  if (isTerminalPhase(state.phase)) {
    const terminalControls = element("div", "controls controls--end")
    terminalControls.append(button("Start another workflow", "button button--secondary", actions.reset))
    panel.append(terminalControls)
  }
  return panel
}

export const renderApp = (
  root: HTMLElement,
  state: WorkflowUiState,
  actions: WorkflowViewActions,
): void => {
  const shell = element("div", "shell")
  const hero = element("header", "hero")
  hero.append(
    element("p", "eyebrow", "OLLAMA COWORK · SPIKE 001"),
    element("h1", undefined, "Document workspace"),
    element(
      "p",
      "lede",
      "A narrow, approval-gated DOCX workflow backed by your configured Ollama model.",
    ),
  )
  shell.append(hero, healthPanel(state), workflowPanel(state, actions))
  if (state.health) {
    shell.append(element("footer", undefined, `Desktop ${state.health.appVersion} · trusted workflow boundary`))
  }
  root.replaceChildren(shell)
}
