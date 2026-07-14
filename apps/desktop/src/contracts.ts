export type ComponentState = "ready" | "configured" | "unavailable" | "unsupported"

export interface ComponentHealth {
  state: ComponentState
  version?: string
  detail: string
}

export interface DesktopHealth {
  appVersion: string
  ready: boolean
  opencode: ComponentHealth
  sandbox: ComponentHealth
  runtimeTools: ComponentHealth
  modelEndpoint: ComponentHealth
}

export interface SelectedDocument {
  selectionId: string
  displayName: string
}

export interface WorkflowReceipt {
  jobId: string
}

export const DESKTOP_HEALTH_EVENT = "desktop://health"
