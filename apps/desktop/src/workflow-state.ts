import type { DesktopHealth, SelectedDocument } from "./contracts"
import type {
  ActionRequest,
  ApplicationError,
  ArtifactMetadata,
  JobStatus,
  WorkflowEvent,
} from "./workflow-contracts"

const MAX_ASSISTANT_MESSAGE_CHARS = 32_768

export type WorkflowPhase =
  | "checking"
  | "blocked"
  | "ready"
  | "starting"
  | "running"
  | "awaiting_approval"
  | "completed"
  | "rejected"
  | "cancelled"
  | "failed"

export interface WorkflowUiState {
  health: DesktopHealth | null
  phase: WorkflowPhase
  selection: SelectedDocument | null
  instruction: string
  jobId: string | null
  assistantText: Array<{ partId: string; text: string }>
  pendingAction: ActionRequest | null
  artifact: ArtifactMetadata | null
  error: ApplicationError | null
  commandPending: boolean
}

export const initialWorkflowState = (): WorkflowUiState => ({
  health: null,
  phase: "checking",
  selection: null,
  instruction: "",
  jobId: null,
  assistantText: [],
  pendingAction: null,
  artifact: null,
  error: null,
  commandPending: false,
})

export const isTerminalPhase = (phase: WorkflowPhase): boolean =>
  phase === "completed" ||
  phase === "rejected" ||
  phase === "cancelled" ||
  phase === "failed"

export const withHealth = (state: WorkflowUiState, health: DesktopHealth): WorkflowUiState => ({
  ...state,
  health,
  phase:
    state.phase === "checking" || state.phase === "blocked" || state.phase === "ready"
      ? health.ready
        ? "ready"
        : "blocked"
      : state.phase,
  error: state.phase === "checking" ? null : state.error,
})

export const withSelection = (
  state: WorkflowUiState,
  selection: SelectedDocument,
): WorkflowUiState => ({
  ...state,
  selection,
  error: null,
  commandPending: false,
})

export const withInstruction = (state: WorkflowUiState, instruction: string): WorkflowUiState => ({
  ...state,
  instruction,
})

export const requestStart = (state: WorkflowUiState): WorkflowUiState => ({
  ...state,
  phase: "starting",
  jobId: null,
  assistantText: [],
  pendingAction: null,
  artifact: null,
  error: null,
  commandPending: true,
})

export const acceptStart = (state: WorkflowUiState, jobId: string): WorkflowUiState => {
  if (state.jobId !== null && state.jobId !== jobId) {
    return withError(state, {
      code: "workflow_job_mismatch",
      message: "The workflow response did not match the active job.",
    })
  }
  return { ...state, jobId, commandPending: false }
}

export const withCommandPending = (
  state: WorkflowUiState,
  commandPending: boolean,
): WorkflowUiState => ({ ...state, commandPending, error: commandPending ? null : state.error })

export const withError = (
  state: WorkflowUiState,
  error: ApplicationError,
  terminal = false,
): WorkflowUiState => ({
  ...state,
  phase: terminal ? "failed" : state.phase,
  error,
  commandPending: false,
})

const phaseForStatus = (status: JobStatus): WorkflowPhase => status

const appendAssistantText = (
  messages: Array<{ partId: string; text: string }>,
  partId: string,
  text: string,
): Array<{ partId: string; text: string }> => {
  const next = messages.map((message) => ({ ...message }))
  const index = next.findIndex((message) => message.partId === partId)
  const target = index === -1 ? next.push({ partId, text: "" }) - 1 : index
  const used = next.reduce((total, message) => total + message.text.length, 0)
  const remaining = MAX_ASSISTANT_MESSAGE_CHARS - used
  if (remaining > 0) next[target].text += text.slice(0, remaining)
  return next
}

export const applyWorkflowEvent = (
  state: WorkflowUiState,
  event: WorkflowEvent,
): WorkflowUiState => {
  const canBindStartingJob = state.phase === "starting" && state.jobId === null
  if (state.jobId !== event.jobId && !canBindStartingJob) return state

  const bound = state.jobId === null ? { ...state, jobId: event.jobId } : state
  switch (event.event) {
    case "status_changed": {
      const terminal = isTerminalPhase(phaseForStatus(event.status))
      return {
        ...bound,
        phase: phaseForStatus(event.status),
        pendingAction: event.status === "running" || terminal ? null : bound.pendingAction,
        commandPending: false,
      }
    }
    case "assistant_text":
      return {
        ...bound,
        assistantText: appendAssistantText(bound.assistantText, event.partId, event.text),
      }
    case "action_requested":
      return {
        ...bound,
        phase: "awaiting_approval",
        pendingAction: event.action,
        commandPending: false,
      }
    case "action_started":
      return { ...bound, phase: "running", pendingAction: null, commandPending: false }
    case "artifact_ready":
      return { ...bound, artifact: event.artifact }
    case "failed":
      return withError(
        bound,
        { code: event.code, message: event.message },
        true,
      )
  }
}

export const resetWorkflow = (state: WorkflowUiState): WorkflowUiState => ({
  ...initialWorkflowState(),
  health: state.health,
  phase: state.health?.ready ? "ready" : "blocked",
})
