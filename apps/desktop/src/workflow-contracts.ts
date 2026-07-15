export type JobStatus =
  | "starting"
  | "running"
  | "awaiting_approval"
  | "completed"
  | "rejected"
  | "cancelled"
  | "failed"

export interface ActionRequest {
  id: string
  title: string
  summary: string
  destructive: boolean
  proposal: ActionProposal
}

export interface DocxSectionRewriteProposal {
  kind: "docx_section_rewrite"
  heading: string
  currentParagraphs: string[]
  replacementParagraphs: string[]
}

export type ActionProposal = DocxSectionRewriteProposal

export interface ArtifactMetadata {
  path: string
  mediaType: string
  sha256: string
}

export type WorkflowEvent =
  | { event: "status_changed"; jobId: string; status: JobStatus }
  | { event: "assistant_text"; jobId: string; partId: string; text: string }
  | { event: "action_requested"; jobId: string; action: ActionRequest }
  | { event: "action_started"; jobId: string; actionId: string }
  | { event: "artifact_ready"; jobId: string; artifact: ArtifactMetadata }
  | { event: "failed"; jobId: string; code: string; message: string }

export interface WorkflowReceipt {
  jobId: string
}

export interface ApplicationError {
  code: string
  message: string
}

export const WORKFLOW_EVENT = "workflow://event"
