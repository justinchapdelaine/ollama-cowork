use crate::BrokerOperation;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, Default)]
pub struct JobCancellation(Arc<AtomicBool>);

impl JobCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub fn validate_workflow_instruction(instruction: &str) -> Result<&str, &'static str> {
    let instruction = instruction.trim();
    if instruction.is_empty() || instruction.len() > 16_384 {
        return Err("instruction is empty or too long");
    }
    Ok(instruction)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "command",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
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
#[serde(rename_all = "camelCase")]
pub struct ActionRequest {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub destructive: bool,
    pub proposal: ActionProposal,
}

/// A bounded, transport-neutral description of the exact change presented to
/// the user. Additional workflow types can add variants without coupling the
/// core controller to a particular frontend.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ActionProposal {
    DocxSectionRewrite {
        heading: String,
        current_paragraphs: Vec<String>,
        replacement_paragraphs: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMetadata {
    pub path: PathBuf,
    pub media_type: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "event",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkflowEvent {
    StatusChanged {
        job_id: String,
        status: JobStatus,
    },
    AssistantText {
        job_id: String,
        part_id: String,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationDecision {
    ApprovedOnce,
    Rejected,
    Cancelled,
}

pub trait MutationAuthorization: Send {
    fn decide(
        &mut self,
        action_id: &str,
        operation: &BrokerOperation,
        decision: MutationDecision,
    ) -> Result<(), String>;
    fn revoke_unconsumed(&mut self, action_id: &str) -> Result<(), String>;
}

pub trait JobCleanup: Send {
    /// Stops all job-scoped model, broker, and sandbox resources and revokes any
    /// capability that has not already been consumed.
    fn terminate(&mut self) -> Result<(), String>;
}

pub struct WorkflowJob<S, A, C> {
    pub session: S,
    pub authorization: A,
    pub cleanup: C,
}

pub type WorkflowJobFor<F> = WorkflowJob<
    <F as WorkflowJobFactory>::Session,
    <F as WorkflowJobFactory>::Authorization,
    <F as WorkflowJobFactory>::Cleanup,
>;

pub trait WorkflowJobFactory: Send + Sized {
    type Session: ModelSession;
    type Authorization: MutationAuthorization;
    type Cleanup: JobCleanup;

    /// Creates all job resources atomically. Implementations must clean up any
    /// partially created resources before returning `Err`.
    fn create(
        &mut self,
        job_id: &str,
        source: &Path,
        cancellation: &JobCancellation,
    ) -> Result<WorkflowJobFor<Self>, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelEvent {
    Text {
        part_id: String,
        text: String,
    },
    ApprovalRequested {
        external_id: String,
        summary: String,
        operation: BrokerOperation,
        proposal: ActionProposal,
    },
    ToolStarted,
    ToolCompleted,
    ArtifactReady(ArtifactMetadata),
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
        assert!(json.contains("jobId"));
        assert!(!json.contains("job_id"));
        assert!(!json.contains("opencode"));
    }

    #[test]
    fn workflow_commands_use_frontend_safe_field_names() {
        let command = WorkflowCommand::ApproveOnce {
            job_id: "job".into(),
            action_id: "action".into(),
        };
        let json = serde_json::to_string(&command).unwrap();
        assert!(json.contains("approve_once"));
        assert!(json.contains("jobId"));
        assert!(json.contains("actionId"));
        assert!(!json.contains("job_id"));
    }

    #[test]
    fn workflow_instruction_validation_is_shared_and_bounded() {
        assert_eq!(
            validate_workflow_instruction("  rewrite  ").unwrap(),
            "rewrite"
        );
        assert!(validate_workflow_instruction("   ").is_err());
        assert!(validate_workflow_instruction(&"x".repeat(16_385)).is_err());
    }
}
