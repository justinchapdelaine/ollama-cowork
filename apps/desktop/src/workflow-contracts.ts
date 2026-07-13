export type JobStatus =
  | "starting"
  | "running"
  | "awaiting_approval"
  | "completed"
  | "rejected"
  | "cancelled"
  | "failed"

export type WorkflowCommand =
  | { command: "start"; source: string; instruction: string }
  | { command: "approve_once"; jobId: string; actionId: string }
  | { command: "reject"; jobId: string; actionId: string }
  | { command: "cancel"; jobId: string }

export interface ActionRequest {
  id: string
  title: string
  summary: string
  destructive: boolean
}

export interface ArtifactMetadata {
  path: string
  mediaType: string
  sha256: string
}

export type WorkflowEvent =
  | { event: "status_changed"; jobId: string; status: JobStatus }
  | { event: "assistant_text"; jobId: string; text: string }
  | { event: "action_requested"; jobId: string; action: ActionRequest }
  | { event: "action_started"; jobId: string; actionId: string }
  | { event: "artifact_ready"; jobId: string; artifact: ArtifactMetadata }
  | { event: "failed"; jobId: string; code: string; message: string }

export interface WorkflowReceipt {
  jobId: string
}

export const WORKFLOW_EVENT = "workflow://event"
