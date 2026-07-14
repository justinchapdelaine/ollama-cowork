import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import {
  DESKTOP_HEALTH_EVENT,
  type DesktopHealth,
  type SelectedDocument,
  type WorkflowReceipt,
} from "./contracts"

export const getDesktopHealth = (): Promise<DesktopHealth> => invoke("get_desktop_health")

export const onDesktopHealth = (handler: (health: DesktopHealth) => void): Promise<UnlistenFn> =>
  listen<DesktopHealth>(DESKTOP_HEALTH_EVENT, ({ payload }) => handler(payload))

export const selectDocxDocument = (): Promise<SelectedDocument | null> =>
  invoke("select_docx_document")

export const startDocxWorkflow = (
  selectionId: string,
  instruction: string,
): Promise<WorkflowReceipt> =>
  invoke("start_docx_workflow", { request: { selectionId, instruction } })
