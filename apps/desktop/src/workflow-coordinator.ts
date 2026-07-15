import { normalizeApplicationError } from "./application-error.ts"
import type { DesktopBridge } from "./desktop-api"
import type { WorkflowEvent } from "./workflow-contracts"
import {
  acceptStart,
  applyWorkflowEvent,
  initialWorkflowState,
  isTerminalPhase,
  requestStart,
  resetWorkflow,
  withCommandPending,
  withError,
  withHealth,
  withInstruction,
  withSelection,
  type WorkflowUiState,
} from "./workflow-state.ts"

const POLL_INTERVAL_MS = 250

export interface WorkflowScheduler {
  schedule(callback: () => void, delayMs: number): unknown
  cancel(handle: unknown): void
}

const browserScheduler: WorkflowScheduler = {
  schedule: (callback, delayMs) => window.setTimeout(callback, delayMs),
  cancel: (handle) => window.clearTimeout(handle as number),
}

export class WorkflowCoordinator {
  private state = initialWorkflowState()
  private readonly unlisten: Array<() => void> = []
  private pollTimer: unknown | null = null
  private pollInFlight = false
  private disposed = false
  private readonly bridge: DesktopBridge
  private readonly render: (state: WorkflowUiState) => void
  private readonly scheduler: WorkflowScheduler

  constructor(
    bridge: DesktopBridge,
    render: (state: WorkflowUiState) => void,
    scheduler: WorkflowScheduler = browserScheduler,
  ) {
    this.bridge = bridge
    this.render = render
    this.scheduler = scheduler
  }

  async initialize(): Promise<void> {
    if (this.disposed) return
    this.render(this.state)
    const established: Array<() => void> = []
    try {
      const unlistenHealth = await this.bridge.onHealth((health) =>
        this.update(withHealth(this.state, health)),
      )
      if (!this.retainListener(unlistenHealth, established)) return
      const unlistenWorkflow = await this.bridge.onWorkflowEvent((event) =>
        this.receiveWorkflowEvent(event),
      )
      if (!this.retainListener(unlistenWorkflow, established)) return
      const health = await this.bridge.getHealth()
      if (!this.disposed) this.update(withHealth(this.state, health))
    } catch (error) {
      this.releaseListeners(established)
      if (!this.disposed) {
        this.update(withError(this.state, normalizeApplicationError(error), true))
      }
    }
  }

  dispose(): void {
    if (this.disposed) return
    this.disposed = true
    this.stopPolling()
    for (const unlisten of this.unlisten.splice(0)) unlisten()
  }

  setInstruction(instruction: string): void {
    this.state = withInstruction(this.state, instruction)
  }

  async selectDocument(): Promise<void> {
    if (this.state.commandPending || isTerminalPhase(this.state.phase)) return
    this.update(withCommandPending(this.state, true))
    try {
      const selection = await this.bridge.selectDocument()
      this.update(
        selection
          ? withSelection(this.state, selection)
          : withCommandPending(this.state, false),
      )
    } catch (error) {
      this.update(withError(this.state, normalizeApplicationError(error)))
    }
  }

  async startWorkflow(): Promise<void> {
    const selection = this.state.selection
    const instruction = this.state.instruction.trim()
    if (!selection || !instruction || this.state.phase !== "ready" || this.state.commandPending) {
      return
    }
    this.update(requestStart(this.state))
    try {
      const receipt = await this.bridge.startWorkflow(selection.selectionId, instruction)
      this.update(acceptStart(this.state, receipt.jobId))
      if (!isTerminalPhase(this.state.phase)) this.schedulePoll(receipt.jobId, 0)
    } catch (error) {
      this.update(withError(this.state, normalizeApplicationError(error), true))
    }
  }

  async approveOnce(): Promise<void> {
    const action = this.state.pendingAction
    const jobId = this.state.jobId
    if (!action || !jobId || this.state.commandPending) return
    await this.runDecision(() => this.bridge.approveOnce(jobId, action.id))
  }

  async reject(): Promise<void> {
    const action = this.state.pendingAction
    const jobId = this.state.jobId
    if (!action || !jobId || this.state.commandPending) return
    await this.runDecision(() => this.bridge.reject(jobId, action.id))
  }

  async cancel(): Promise<void> {
    const jobId = this.state.jobId
    if (!jobId || isTerminalPhase(this.state.phase) || this.state.commandPending) return
    await this.runDecision(() => this.bridge.cancel(jobId))
  }

  reset(): void {
    this.stopPolling()
    this.update(resetWorkflow(this.state))
  }

  private async runDecision(command: () => Promise<unknown>): Promise<void> {
    this.update(withCommandPending(this.state, true))
    try {
      await command()
      this.update(withCommandPending(this.state, false))
    } catch (error) {
      this.update(withError(this.state, normalizeApplicationError(error)))
    }
  }

  private receiveWorkflowEvent(event: WorkflowEvent): void {
    this.update(applyWorkflowEvent(this.state, event))
    if (isTerminalPhase(this.state.phase)) this.stopPolling()
  }

  private schedulePoll(jobId: string, delay = POLL_INTERVAL_MS): void {
    if (this.disposed || this.pollTimer !== null || isTerminalPhase(this.state.phase)) return
    this.pollTimer = this.scheduler.schedule(() => {
      this.pollTimer = null
      void this.poll(jobId)
    }, delay)
  }

  private async poll(jobId: string): Promise<void> {
    if (
      this.disposed ||
      this.pollInFlight ||
      isTerminalPhase(this.state.phase) ||
      this.state.jobId !== jobId
    ) return
    this.pollInFlight = true
    try {
      await this.bridge.poll(jobId)
    } catch (error) {
      const normalized = normalizeApplicationError(error)
      if (
        normalized.code !== "workflow_starting" &&
        !isTerminalPhase(this.state.phase)
      ) {
        this.update(withError(this.state, normalized, true))
      }
    } finally {
      this.pollInFlight = false
    }
    if (!this.disposed && !isTerminalPhase(this.state.phase)) this.schedulePoll(jobId)
  }

  private stopPolling(): void {
    if (this.pollTimer !== null) this.scheduler.cancel(this.pollTimer)
    this.pollTimer = null
  }

  private update(state: WorkflowUiState): void {
    if (this.disposed) return
    this.state = state
    this.render(state)
  }

  private retainListener(
    unlisten: () => void,
    established: Array<() => void>,
  ): boolean {
    if (this.disposed) {
      unlisten()
      return false
    }
    this.unlisten.push(unlisten)
    established.push(unlisten)
    return true
  }

  private releaseListeners(listeners: Array<() => void>): void {
    for (const unlisten of listeners.splice(0)) {
      const index = this.unlisten.indexOf(unlisten)
      if (index >= 0) this.unlisten.splice(index, 1)
      unlisten()
    }
  }
}
