import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import {
  DESKTOP_HEALTH_EVENT,
  type DesktopHealth,
  type SelectedDocument,
} from "./contracts"
import {
  WORKFLOW_EVENT,
  type WorkflowEvent,
  type WorkflowReceipt,
} from "./workflow-contracts"

export interface DesktopBridge {
  getHealth(): Promise<DesktopHealth>
  onHealth(handler: (health: DesktopHealth) => void): Promise<UnlistenFn>
  onWorkflowEvent(handler: (event: WorkflowEvent) => void): Promise<UnlistenFn>
  selectDocument(): Promise<SelectedDocument | null>
  startWorkflow(selectionId: string, instruction: string): Promise<WorkflowReceipt>
  approveOnce(jobId: string, actionId: string): Promise<WorkflowReceipt>
  reject(jobId: string, actionId: string): Promise<WorkflowReceipt>
  cancel(jobId: string): Promise<WorkflowReceipt>
  poll(jobId: string): Promise<void>
}

const actionRequest = (jobId: string, actionId: string) => ({ request: { jobId, actionId } })
const jobRequest = (jobId: string) => ({ request: { jobId } })

export const tauriDesktopBridge: DesktopBridge = {
  getHealth: () => invoke("get_desktop_health"),
  onHealth: (handler) =>
    listen<DesktopHealth>(DESKTOP_HEALTH_EVENT, ({ payload }) => handler(payload)),
  onWorkflowEvent: (handler) =>
    listen<WorkflowEvent>(WORKFLOW_EVENT, ({ payload }) => handler(payload)),
  selectDocument: () => invoke("select_docx_document"),
  startWorkflow: (selectionId, instruction) =>
    invoke("start_docx_workflow", { request: { selectionId, instruction } }),
  approveOnce: (jobId, actionId) =>
    invoke("approve_docx_action_once", actionRequest(jobId, actionId)),
  reject: (jobId, actionId) => invoke("reject_docx_action", actionRequest(jobId, actionId)),
  cancel: (jobId) => invoke("cancel_docx_workflow", jobRequest(jobId)),
  poll: (jobId) => invoke("poll_docx_workflow", jobRequest(jobId)),
}
