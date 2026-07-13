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
  modelEndpoint: ComponentHealth
}

export const DESKTOP_HEALTH_EVENT = "desktop://health"
