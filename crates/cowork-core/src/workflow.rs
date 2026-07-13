use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WorkflowCommand {
    Start {
        source: PathBuf,
        instruction: String,
    },
    ApproveOnce {
        job_id: String,
        action_id: String,
    },
    Reject {
        job_id: String,
        action_id: String,
    },
    Cancel {
        job_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Starting,
    Running,
    AwaitingApproval,
    Completed,
    Rejected,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub destructive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    pub path: PathBuf,
    pub media_type: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WorkflowEvent {
    StatusChanged {
        job_id: String,
        status: JobStatus,
    },
    AssistantText {
        job_id: String,
        text: String,
    },
    ActionRequested {
        job_id: String,
        action: ActionRequest,
    },
    ActionStarted {
        job_id: String,
        action_id: String,
    },
    ArtifactReady {
        job_id: String,
        artifact: ArtifactMetadata,
    },
    Failed {
        job_id: String,
        code: String,
        message: String,
    },
}

pub trait WorkflowEventSink: Send + Sync {
    fn emit(&self, event: WorkflowEvent);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelEvent {
    Text(String),
    ApprovalRequested {
        external_id: String,
        tool: String,
        summary: String,
    },
    ToolStarted {
        external_id: String,
        tool: String,
    },
    ToolCompleted {
        external_id: String,
        tool: String,
        output: String,
    },
    Idle,
    Failed {
        code: String,
        message: String,
    },
}

pub trait ModelSession: Send {
    fn submit(&mut self, instruction: &str) -> Result<(), String>;
    fn decide(&mut self, external_id: &str, approved_once: bool) -> Result<(), String>;
    fn cancel(&mut self) -> Result<(), String>;
    fn next_event(&mut self) -> Result<Option<ModelEvent>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workflow_events_are_transport_neutral() {
        let event = WorkflowEvent::StatusChanged {
            job_id: "job".into(),
            status: JobStatus::Running,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("status_changed"));
        assert!(!json.contains("opencode"));
    }
}
