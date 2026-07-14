use crate::{
    ActionRequest, BrokerOperation, JobCancellation, JobCleanup, JobStatus, ModelEvent,
    ModelSession, MutationAuthorization, MutationDecision, WorkflowCommand, WorkflowEvent,
    WorkflowEventSink, WorkflowJobFactory,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowReceipt {
    pub job_id: String,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum WorkflowError {
    #[error("invalid workflow request: {0}")]
    InvalidRequest(&'static str),
    #[error("unknown workflow job")]
    UnknownJob,
    #[error("workflow job is already terminal")]
    TerminalJob,
    #[error("approval action does not match the pending action")]
    ActionMismatch,
    #[error("workflow integration failed: {0}")]
    Model(&'static str),
}

const POLL_BATCH_SIZE: usize = 256;
const MAX_JOB_EVENTS: usize = 16_384;

#[derive(Clone, Debug)]
struct ActiveAction {
    id: String,
    permission_id: String,
    operation: BrokerOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JobPhase {
    Submitted,
    AwaitingApproval,
    Approved,
    ToolRunning,
    ToolCompleted,
    ArtifactReady,
}

struct ActiveJob<S, A, C> {
    session: S,
    authorization: A,
    cleanup: C,
    status: JobStatus,
    phase: JobPhase,
    action: Option<ActiveAction>,
    events_seen: usize,
}

type ActiveJobFor<F> = ActiveJob<
    <F as WorkflowJobFactory>::Session,
    <F as WorkflowJobFactory>::Authorization,
    <F as WorkflowJobFactory>::Cleanup,
>;

pub struct WorkflowController<F, E>
where
    F: WorkflowJobFactory,
    E: WorkflowEventSink,
{
    factory: F,
    events: E,
    jobs: HashMap<String, ActiveJobFor<F>>,
    finished_jobs: HashSet<String>,
    next_job: u64,
    next_action: u64,
}

impl<F, E> WorkflowController<F, E>
where
    F: WorkflowJobFactory,
    E: WorkflowEventSink,
{
    pub fn new(factory: F, events: E) -> Self {
        Self {
            factory,
            events,
            jobs: HashMap::new(),
            finished_jobs: HashSet::new(),
            next_job: 1,
            next_action: 1,
        }
    }

    pub fn handle(&mut self, command: WorkflowCommand) -> Result<WorkflowReceipt, WorkflowError> {
        match command {
            WorkflowCommand::Start {
                source,
                instruction,
            } => {
                let job_id = self.reserve_job_id();
                self.start_reserved(job_id, source, instruction, &JobCancellation::default())
            }
            WorkflowCommand::ApproveOnce { job_id, action_id } => {
                self.handle_approve_once(job_id, action_id)
            }
            WorkflowCommand::Reject { job_id, action_id } => self.handle_reject(job_id, action_id),
            WorkflowCommand::Cancel { job_id } => self.handle_cancel(job_id),
        }
    }

    pub fn reserve_job_id(&mut self) -> String {
        let job_id = format!("job-{}", self.next_job);
        self.next_job += 1;
        job_id
    }

    pub fn start_reserved(
        &mut self,
        job_id: String,
        source: std::path::PathBuf,
        instruction: String,
        cancellation: &JobCancellation,
    ) -> Result<WorkflowReceipt, WorkflowError> {
        if source
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("docx"))
            != Some(true)
        {
            return Err(WorkflowError::InvalidRequest("source must be a DOCX"));
        }
        let instruction = crate::validate_workflow_instruction(&instruction)
            .map_err(WorkflowError::InvalidRequest)?;
        self.events.emit(WorkflowEvent::StatusChanged {
            job_id: job_id.clone(),
            status: JobStatus::Starting,
        });
        if cancellation.is_cancelled() {
            return self.finish_cancelled_start(job_id);
        }
        let mut resources = match self.factory.create(&job_id, &source, cancellation) {
            Ok(resources) => resources,
            Err(_message) => {
                if cancellation.is_cancelled() {
                    return self.finish_cancelled_start(job_id);
                }
                self.emit_failure(
                    &job_id,
                    "workflow_job_create_failed",
                    "The document workflow could not be initialized.",
                );
                return Err(WorkflowError::Model("workflow initialization failed"));
            }
        };
        if cancellation.is_cancelled() {
            let _ = resources.cleanup.terminate();
            return self.finish_cancelled_start(job_id);
        }
        if let Err(_message) = resources.session.submit(instruction) {
            let _ = resources.cleanup.terminate();
            self.emit_failure(
                &job_id,
                "model_submission_failed",
                "The instruction could not be submitted to the model.",
            );
            return Err(WorkflowError::Model("model submission failed"));
        }
        if cancellation.is_cancelled() {
            let _ = resources.session.cancel();
            let _ = resources.cleanup.terminate();
            return self.finish_cancelled_start(job_id);
        }
        self.jobs.insert(
            job_id.clone(),
            ActiveJob {
                session: resources.session,
                authorization: resources.authorization,
                cleanup: resources.cleanup,
                status: JobStatus::Running,
                phase: JobPhase::Submitted,
                action: None,
                events_seen: 0,
            },
        );
        self.events.emit(WorkflowEvent::StatusChanged {
            job_id: job_id.clone(),
            status: JobStatus::Running,
        });
        Ok(WorkflowReceipt { job_id })
    }

    fn handle_approve_once(
        &mut self,
        job_id: String,
        action_id: String,
    ) -> Result<WorkflowReceipt, WorkflowError> {
        let (permission_id, operation) = self.pending_action(&job_id, &action_id)?;
        if let Err(_message) = self.active_job_mut(&job_id)?.authorization.decide(
            &action_id,
            &operation,
            MutationDecision::ApprovedOnce,
        ) {
            let _ = self.active_job_mut(&job_id)?.session.cancel();
            self.fail_active_job(
                &job_id,
                "broker_approval_failed",
                "The trusted document boundary did not accept the approval.",
            )?;
            return Err(WorkflowError::Model("broker approval failed"));
        }
        if let Err(_message) = self
            .active_job_mut(&job_id)?
            .session
            .decide(&permission_id, true)
        {
            let job = self.active_job_mut(&job_id)?;
            let _ = job.session.cancel();
            let _ = job.authorization.revoke_unconsumed(&action_id);
            self.fail_active_job(
                &job_id,
                "model_approval_failed",
                "The model permission could not be completed; authorization was revoked.",
            )?;
            return Err(WorkflowError::Model("model approval failed"));
        }
        let job = self.active_job_mut(&job_id)?;
        job.status = JobStatus::Running;
        job.phase = JobPhase::Approved;
        self.events.emit(WorkflowEvent::StatusChanged {
            job_id: job_id.clone(),
            status: JobStatus::Running,
        });
        Ok(WorkflowReceipt { job_id })
    }

    fn handle_reject(
        &mut self,
        job_id: String,
        action_id: String,
    ) -> Result<WorkflowReceipt, WorkflowError> {
        let (permission_id, operation) = self.pending_action(&job_id, &action_id)?;
        if let Err(_message) = self.active_job_mut(&job_id)?.authorization.decide(
            &action_id,
            &operation,
            MutationDecision::Rejected,
        ) {
            let _ = self.active_job_mut(&job_id)?.session.cancel();
            self.fail_active_job(
                &job_id,
                "broker_rejection_failed",
                "The trusted document boundary could not record the rejection.",
            )?;
            return Err(WorkflowError::Model("broker rejection failed"));
        }
        if let Err(_message) = self
            .active_job_mut(&job_id)?
            .session
            .decide(&permission_id, false)
        {
            self.fail_active_job(
                &job_id,
                "model_rejection_failed",
                "The model permission could not be rejected cleanly.",
            )?;
            return Err(WorkflowError::Model("model rejection failed"));
        }
        self.finish_active_job(&job_id, JobStatus::Rejected)?;
        Ok(WorkflowReceipt { job_id })
    }

    fn handle_cancel(&mut self, job_id: String) -> Result<WorkflowReceipt, WorkflowError> {
        let action_id = self
            .active_job_mut(&job_id)?
            .action
            .as_ref()
            .map(|action| action.id.clone())
            .unwrap_or_else(|| "workflow-cancel".into());
        let operation = self
            .active_job_mut(&job_id)?
            .action
            .as_ref()
            .map(|action| action.operation.clone())
            .unwrap_or(BrokerOperation::Inspect);
        let authorization_result = self.active_job_mut(&job_id)?.authorization.decide(
            &action_id,
            &operation,
            MutationDecision::Cancelled,
        );
        let session_result = self.active_job_mut(&job_id)?.session.cancel();
        if let Err(_message) = authorization_result.or(session_result) {
            self.fail_active_job(
                &job_id,
                "workflow_cancellation_failed",
                "The document workflow could not be cancelled cleanly.",
            )?;
            return Err(WorkflowError::Model("workflow cancellation failed"));
        }
        self.finish_active_job(&job_id, JobStatus::Cancelled)?;
        Ok(WorkflowReceipt { job_id })
    }

    pub fn is_terminal_job(&self, job_id: &str) -> bool {
        self.finished_jobs.contains(job_id)
    }

    fn finish_cancelled_start(&mut self, job_id: String) -> Result<WorkflowReceipt, WorkflowError> {
        self.finished_jobs.insert(job_id.clone());
        self.events.emit(WorkflowEvent::StatusChanged {
            job_id: job_id.clone(),
            status: JobStatus::Cancelled,
        });
        Ok(WorkflowReceipt { job_id })
    }

    pub fn poll(&mut self, job_id: &str) -> Result<(), WorkflowError> {
        for _ in 0..POLL_BATCH_SIZE {
            let event = {
                let job = self.active_job_mut(job_id)?;
                job.session.next_event()
            };
            let event = match event {
                Ok(event) => event,
                Err(_message) => {
                    self.fail_active_job(
                        job_id,
                        "model_event_failed",
                        "The model event stream failed.",
                    )?;
                    return Err(WorkflowError::Model("model event stream failed"));
                }
            };
            let Some(event) = event else { return Ok(()) };
            let job = self.active_job_mut(job_id)?;
            job.events_seen += 1;
            if job.events_seen > MAX_JOB_EVENTS {
                self.fail_active_job(
                    job_id,
                    "workflow_event_limit_exceeded",
                    "The model produced too many workflow events.",
                )?;
                return Ok(());
            }
            match event {
                ModelEvent::Text(text) => self.events.emit(WorkflowEvent::AssistantText {
                    job_id: job_id.into(),
                    text,
                }),
                ModelEvent::ApprovalRequested {
                    external_id,
                    summary,
                    operation,
                } => {
                    let action_id = format!("action-{}", self.next_action);
                    self.next_action += 1;
                    let job = self.active_job_mut(job_id)?;
                    if job.action.is_some() || job.phase != JobPhase::Submitted {
                        self.fail_active_job(
                            job_id,
                            "invalid_workflow_transition",
                            "approval was requested outside the submitted workflow phase",
                        )?;
                        return Ok(());
                    }
                    job.action = Some(ActiveAction {
                        id: action_id.clone(),
                        permission_id: external_id,
                        operation,
                    });
                    job.status = JobStatus::AwaitingApproval;
                    job.phase = JobPhase::AwaitingApproval;
                    self.events.emit(WorkflowEvent::StatusChanged {
                        job_id: job_id.into(),
                        status: JobStatus::AwaitingApproval,
                    });
                    self.events.emit(WorkflowEvent::ActionRequested {
                        job_id: job_id.into(),
                        action: ActionRequest {
                            id: action_id,
                            title: "Create a revised DOCX copy?".into(),
                            summary,
                            destructive: false,
                        },
                    });
                    return Ok(());
                }
                ModelEvent::ToolStarted => {
                    let action_id = {
                        let job = self.active_job_mut(job_id)?;
                        if job.phase == JobPhase::Approved {
                            job.action.as_ref().map(|action| action.id.clone())
                        } else {
                            None
                        }
                    };
                    let Some(action_id) = action_id else {
                        self.fail_active_job(
                            job_id,
                            "invalid_workflow_transition",
                            "DOCX tool started without an approved action",
                        )?;
                        return Ok(());
                    };
                    self.active_job_mut(job_id)?.phase = JobPhase::ToolRunning;
                    self.events.emit(WorkflowEvent::ActionStarted {
                        job_id: job_id.into(),
                        action_id,
                    });
                }
                ModelEvent::ToolCompleted => {
                    let job = self.active_job_mut(job_id)?;
                    if job.phase != JobPhase::ToolRunning {
                        self.fail_active_job(
                            job_id,
                            "invalid_workflow_transition",
                            "DOCX tool completed before it started",
                        )?;
                        return Ok(());
                    }
                    job.phase = JobPhase::ToolCompleted;
                }
                ModelEvent::ArtifactReady(artifact) => {
                    let job = self.active_job_mut(job_id)?;
                    if job.phase != JobPhase::ToolCompleted {
                        self.fail_active_job(
                            job_id,
                            "invalid_workflow_transition",
                            "a revised artifact was reported before tool completion",
                        )?;
                        return Ok(());
                    }
                    job.phase = JobPhase::ArtifactReady;
                    self.events.emit(WorkflowEvent::ArtifactReady {
                        job_id: job_id.into(),
                        artifact,
                    });
                }
                ModelEvent::Idle => {
                    let job = self.active_job_mut(job_id)?;
                    if job.phase != JobPhase::ArtifactReady {
                        self.fail_active_job(
                            job_id,
                            "incomplete_docx_workflow",
                            "opencode became idle before a validated revised DOCX was ready",
                        )?;
                        return Ok(());
                    }
                    self.finish_active_job(job_id, JobStatus::Completed)?;
                    return Ok(());
                }
                ModelEvent::Failed { code, message } => {
                    self.fail_active_job(job_id, &code, &message)?;
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    pub fn forget_terminal_job(&mut self, job_id: &str) -> bool {
        self.finished_jobs.remove(job_id)
    }

    /// Cancels and cleans up every active job without stopping after one failure.
    ///
    /// Desktop lifecycle adapters call this before process exit so supervised
    /// broker, opencode, and sandbox trees cannot be left behind.
    pub fn shutdown(&mut self) -> Result<(), WorkflowError> {
        let jobs = std::mem::take(&mut self.jobs);
        let mut failed = false;
        for (job_id, mut job) in jobs {
            let (action_id, operation) = job
                .action
                .as_ref()
                .map(|action| (action.id.clone(), action.operation.clone()))
                .unwrap_or_else(|| ("workflow-shutdown".into(), BrokerOperation::Inspect));
            let authorization_failed = job
                .authorization
                .decide(&action_id, &operation, MutationDecision::Cancelled)
                .is_err();
            let session_failed = job.session.cancel().is_err();
            let cleanup_failed = job.cleanup.terminate().is_err();
            let job_failed = authorization_failed || session_failed || cleanup_failed;
            failed |= job_failed;
            self.finished_jobs.insert(job_id.clone());
            if job_failed {
                self.emit_failure(
                    &job_id,
                    "runtime_shutdown_failed",
                    "The isolated workflow runtime did not stop cleanly.",
                );
            } else {
                self.events.emit(WorkflowEvent::StatusChanged {
                    job_id,
                    status: JobStatus::Cancelled,
                });
            }
        }
        if failed {
            Err(WorkflowError::Model("runtime shutdown failed"))
        } else {
            Ok(())
        }
    }

    fn pending_action(
        &mut self,
        job_id: &str,
        action_id: &str,
    ) -> Result<(String, BrokerOperation), WorkflowError> {
        let job = self.active_job_mut(job_id)?;
        let action = job.action.as_ref().ok_or(WorkflowError::ActionMismatch)?;
        if action.id != action_id
            || job.status != JobStatus::AwaitingApproval
            || job.phase != JobPhase::AwaitingApproval
        {
            return Err(WorkflowError::ActionMismatch);
        }
        Ok((action.permission_id.clone(), action.operation.clone()))
    }

    fn finish_active_job(&mut self, job_id: &str, status: JobStatus) -> Result<(), WorkflowError> {
        let mut job = self.jobs.remove(job_id).ok_or(WorkflowError::UnknownJob)?;
        if let Err(_message) = job.cleanup.terminate() {
            self.finished_jobs.insert(job_id.into());
            self.emit_failure(
                job_id,
                "runtime_cleanup_failed",
                "The isolated workflow runtime did not stop cleanly.",
            );
            return Err(WorkflowError::Model("runtime cleanup failed"));
        }
        self.finished_jobs.insert(job_id.into());
        self.events.emit(WorkflowEvent::StatusChanged {
            job_id: job_id.into(),
            status,
        });
        Ok(())
    }

    fn fail_active_job(
        &mut self,
        job_id: &str,
        code: &str,
        message: &str,
    ) -> Result<(), WorkflowError> {
        let mut job = self.jobs.remove(job_id).ok_or(WorkflowError::UnknownJob)?;
        let _ = job.session.cancel();
        let _ = job.cleanup.terminate();
        self.finished_jobs.insert(job_id.into());
        self.emit_failure(job_id, code, message);
        Ok(())
    }

    fn emit_failure(&self, job_id: &str, code: &str, message: &str) {
        self.events.emit(WorkflowEvent::StatusChanged {
            job_id: job_id.into(),
            status: JobStatus::Failed,
        });
        self.events.emit(WorkflowEvent::Failed {
            job_id: job_id.into(),
            code: code.into(),
            message: message.into(),
        });
    }

    fn active_job_mut(&mut self, job_id: &str) -> Result<&mut ActiveJobFor<F>, WorkflowError> {
        if self.finished_jobs.contains(job_id) {
            return Err(WorkflowError::TerminalJob);
        }
        let job = self.jobs.get_mut(job_id).ok_or(WorkflowError::UnknownJob)?;
        if matches!(
            job.status,
            JobStatus::Completed | JobStatus::Rejected | JobStatus::Cancelled | JobStatus::Failed
        ) {
            return Err(WorkflowError::TerminalJob);
        }
        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactMetadata, ModelEvent};
    use std::{
        collections::VecDeque,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    struct Session {
        events: VecDeque<ModelEvent>,
        decisions: Arc<Mutex<Vec<String>>>,
        cancelled: Arc<Mutex<bool>>,
        submit_error: Option<String>,
        decide_error: Option<String>,
        event_error: Option<String>,
    }
    impl ModelSession for Session {
        fn submit(&mut self, _: &str) -> Result<(), String> {
            self.submit_error.take().map_or(Ok(()), Err)
        }
        fn decide(&mut self, id: &str, approved: bool) -> Result<(), String> {
            if let Some(message) = self.decide_error.take() {
                return Err(message);
            }
            self.decisions
                .lock()
                .unwrap()
                .push(format!("model:{id}:{approved}"));
            Ok(())
        }
        fn cancel(&mut self) -> Result<(), String> {
            *self.cancelled.lock().unwrap() = true;
            Ok(())
        }
        fn next_event(&mut self) -> Result<Option<ModelEvent>, String> {
            if let Some(message) = self.event_error.take() {
                return Err(message);
            }
            Ok(self.events.pop_front())
        }
    }
    struct Authorization(Arc<Mutex<Vec<String>>>);
    impl MutationAuthorization for Authorization {
        fn decide(
            &mut self,
            action_id: &str,
            _: &BrokerOperation,
            decision: MutationDecision,
        ) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .push(format!("authorization:{action_id}:{decision:?}"));
            Ok(())
        }
        fn revoke_unconsumed(&mut self, action_id: &str) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .push(format!("authorization:{action_id}:revoked"));
            Ok(())
        }
    }
    struct Cleanup(Arc<Mutex<usize>>);
    impl JobCleanup for Cleanup {
        fn terminate(&mut self) -> Result<(), String> {
            *self.0.lock().unwrap() += 1;
            Ok(())
        }
    }
    struct Factory {
        events: Option<VecDeque<ModelEvent>>,
        decisions: Arc<Mutex<Vec<String>>>,
        cancelled: Arc<Mutex<bool>>,
        create_error: Option<String>,
        submit_error: Option<String>,
        decide_error: Option<String>,
        event_error: Option<String>,
        cleanup_calls: Arc<Mutex<usize>>,
    }
    impl WorkflowJobFactory for Factory {
        type Session = Session;
        type Authorization = Authorization;
        type Cleanup = Cleanup;
        fn create(
            &mut self,
            _: &str,
            _: &std::path::Path,
            _: &JobCancellation,
        ) -> Result<crate::WorkflowJobFor<Self>, String> {
            if let Some(message) = self.create_error.take() {
                return Err(message);
            }
            Ok(crate::WorkflowJob {
                session: Session {
                    events: self.events.take().unwrap_or_default(),
                    decisions: self.decisions.clone(),
                    cancelled: self.cancelled.clone(),
                    submit_error: self.submit_error.take(),
                    decide_error: self.decide_error.take(),
                    event_error: self.event_error.take(),
                },
                authorization: Authorization(self.decisions.clone()),
                cleanup: Cleanup(self.cleanup_calls.clone()),
            })
        }
    }
    #[derive(Clone)]
    struct Sink(Arc<Mutex<Vec<WorkflowEvent>>>);
    impl WorkflowEventSink for Sink {
        fn emit(&self, event: WorkflowEvent) {
            self.0.lock().unwrap().push(event)
        }
    }
    type ControllerFixture = (
        WorkflowController<Factory, Sink>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<bool>>,
        Arc<Mutex<Vec<WorkflowEvent>>>,
    );
    fn controller(events: Vec<ModelEvent>) -> ControllerFixture {
        let decisions = Arc::new(Mutex::new(Vec::new()));
        let cancelled = Arc::new(Mutex::new(false));
        let emitted = Arc::new(Mutex::new(Vec::new()));
        (
            WorkflowController::new(
                Factory {
                    events: Some(events.into()),
                    decisions: decisions.clone(),
                    cancelled: cancelled.clone(),
                    create_error: None,
                    submit_error: None,
                    decide_error: None,
                    event_error: None,
                    cleanup_calls: Arc::new(Mutex::new(0)),
                },
                Sink(emitted.clone()),
            ),
            decisions,
            cancelled,
            emitted,
        )
    }
    fn start(controller: &mut WorkflowController<Factory, Sink>) -> String {
        controller
            .handle(WorkflowCommand::Start {
                source: PathBuf::from("input.docx"),
                instruction: "Rewrite the executive summary".into(),
            })
            .unwrap()
            .job_id
    }
    fn rewrite_operation() -> BrokerOperation {
        BrokerOperation::RewriteSection {
            heading: "Summary".into(),
            replacement_paragraphs: vec!["Revised".into()],
        }
    }

    #[test]
    fn approval_is_correlated_and_forwarded_once() {
        let (mut controller, decisions, _, emitted) =
            controller(vec![ModelEvent::ApprovalRequested {
                external_id: "external".into(),
                summary: "Create input.revised.docx".into(),
                operation: rewrite_operation(),
            }]);
        let job = start(&mut controller);
        controller.poll(&job).unwrap();
        assert_eq!(
            controller.handle(WorkflowCommand::ApproveOnce {
                job_id: job.clone(),
                action_id: "wrong".into()
            }),
            Err(WorkflowError::ActionMismatch)
        );
        controller
            .handle(WorkflowCommand::ApproveOnce {
                job_id: job,
                action_id: "action-1".into(),
            })
            .unwrap();
        assert_eq!(
            *decisions.lock().unwrap(),
            vec!["authorization:action-1:ApprovedOnce", "model:external:true"]
        );
        assert!(
            emitted
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, WorkflowEvent::ActionRequested { .. }))
        );
    }

    #[test]
    fn reject_and_cancel_are_terminal() {
        let (mut rejected, decisions, _, _) = controller(vec![ModelEvent::ApprovalRequested {
            external_id: "external".into(),
            summary: "rewrite".into(),
            operation: rewrite_operation(),
        }]);
        let job = start(&mut rejected);
        rejected.poll(&job).unwrap();
        rejected
            .handle(WorkflowCommand::Reject {
                job_id: job.clone(),
                action_id: "action-1".into(),
            })
            .unwrap();
        assert_eq!(
            *decisions.lock().unwrap(),
            vec!["authorization:action-1:Rejected", "model:external:false"]
        );
        assert_eq!(rejected.poll(&job), Err(WorkflowError::TerminalJob));

        let (mut cancelled_controller, _, cancelled, _) = controller(vec![]);
        let cancelled_job = start(&mut cancelled_controller);
        cancelled_controller
            .handle(WorkflowCommand::Cancel {
                job_id: cancelled_job.clone(),
            })
            .unwrap();
        assert!(*cancelled.lock().unwrap());
        assert_eq!(
            cancelled_controller.poll(&cancelled_job),
            Err(WorkflowError::TerminalJob)
        );
    }

    #[test]
    fn artifact_and_completion_are_normalized() {
        let artifact = ArtifactMetadata {
            path: "input.revised.docx".into(),
            media_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                .into(),
            sha256: "abc".into(),
        };
        let (mut controller, _, _, emitted) = controller(vec![
            ModelEvent::ApprovalRequested {
                external_id: "permission-id".into(),
                summary: "rewrite".into(),
                operation: rewrite_operation(),
            },
            ModelEvent::ToolStarted,
            ModelEvent::ToolCompleted,
            ModelEvent::ArtifactReady(artifact.clone()),
            ModelEvent::Idle,
        ]);
        let job = start(&mut controller);
        controller.poll(&job).unwrap();
        controller
            .handle(WorkflowCommand::ApproveOnce {
                job_id: job.clone(),
                action_id: "action-1".into(),
            })
            .unwrap();
        controller.poll(&job).unwrap();
        let events = emitted.lock().unwrap();
        assert!(events.iter().any(|event| matches!(event, WorkflowEvent::ArtifactReady { artifact: value, .. } if value == &artifact)));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::StatusChanged {
                status: JobStatus::Completed,
                ..
            }
        )));
    }

    #[test]
    fn idle_without_a_validated_artifact_fails_closed() {
        let (mut controller, _, _, emitted) = controller(vec![ModelEvent::Idle]);
        let job = start(&mut controller);
        controller.poll(&job).unwrap();

        let events = emitted.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::StatusChanged {
                status: JobStatus::Failed,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::Failed { code, .. } if code == "incomplete_docx_workflow"
        )));
        drop(events);
        assert_eq!(controller.poll(&job), Err(WorkflowError::TerminalJob));
    }

    #[test]
    fn adapter_failures_update_status_and_emit_details() {
        let (mut controller, _, _, emitted) = controller(vec![ModelEvent::Failed {
            code: "unexpected_tool".into(),
            message: "denied".into(),
        }]);
        let job = start(&mut controller);
        controller.poll(&job).unwrap();

        let events = emitted.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::StatusChanged {
                status: JobStatus::Failed,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::Failed { code, message, .. }
                if code == "unexpected_tool" && message == "denied"
        )));
    }

    #[test]
    fn startup_and_poll_errors_are_normalized_as_terminal_failures() {
        for (create_error, submit_error, event_error, expected_code) in [
            (Some("create"), None, None, "workflow_job_create_failed"),
            (None, Some("submit"), None, "model_submission_failed"),
            (None, None, Some("stream"), "model_event_failed"),
        ] {
            let emitted = Arc::new(Mutex::new(Vec::new()));
            let mut controller = WorkflowController::new(
                Factory {
                    events: Some(VecDeque::new()),
                    decisions: Arc::new(Mutex::new(Vec::new())),
                    cancelled: Arc::new(Mutex::new(false)),
                    create_error: create_error.map(str::to_owned),
                    submit_error: submit_error.map(str::to_owned),
                    decide_error: None,
                    event_error: event_error.map(str::to_owned),
                    cleanup_calls: Arc::new(Mutex::new(0)),
                },
                Sink(emitted.clone()),
            );
            let result = controller.handle(WorkflowCommand::Start {
                source: "input.docx".into(),
                instruction: "rewrite".into(),
            });
            if event_error.is_some() {
                let job = result.unwrap().job_id;
                assert!(matches!(
                    controller.poll(&job),
                    Err(WorkflowError::Model(_))
                ));
                assert_eq!(controller.poll(&job), Err(WorkflowError::TerminalJob));
            } else {
                assert!(matches!(result, Err(WorkflowError::Model(_))));
            }
            let events = emitted.lock().unwrap();
            assert!(events.iter().any(|event| matches!(
                event,
                WorkflowEvent::Failed { code, .. } if code == expected_code
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                WorkflowEvent::StatusChanged {
                    status: JobStatus::Failed,
                    ..
                }
            )));
        }
    }

    #[test]
    fn failed_model_approval_revokes_the_broker_authorization() {
        let decisions = Arc::new(Mutex::new(Vec::new()));
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let mut controller = WorkflowController::new(
            Factory {
                events: Some(
                    vec![ModelEvent::ApprovalRequested {
                        external_id: "permission".into(),
                        summary: "rewrite".into(),
                        operation: rewrite_operation(),
                    }]
                    .into(),
                ),
                decisions: decisions.clone(),
                cancelled: Arc::new(Mutex::new(false)),
                create_error: None,
                submit_error: None,
                decide_error: Some("reply failed".into()),
                event_error: None,
                cleanup_calls: Arc::new(Mutex::new(0)),
            },
            Sink(emitted.clone()),
        );
        let job = start(&mut controller);
        controller.poll(&job).unwrap();
        assert_eq!(
            controller.handle(WorkflowCommand::ApproveOnce {
                job_id: job.clone(),
                action_id: "action-1".into(),
            }),
            Err(WorkflowError::Model("model approval failed"))
        );
        assert_eq!(
            *decisions.lock().unwrap(),
            vec![
                "authorization:action-1:ApprovedOnce",
                "authorization:action-1:revoked"
            ]
        );
        assert_eq!(controller.poll(&job), Err(WorkflowError::TerminalJob));
        assert!(emitted.lock().unwrap().iter().any(|event| matches!(
            event,
            WorkflowEvent::Failed { code, .. } if code == "model_approval_failed"
        )));
    }

    #[test]
    fn poll_budget_yields_without_failing_a_verbose_job() {
        let (mut controller, _, _, emitted) = controller(
            (0..300)
                .map(|index| ModelEvent::Text(index.to_string()))
                .collect(),
        );
        let job = start(&mut controller);
        controller.poll(&job).unwrap();
        controller.poll(&job).unwrap();
        let events = emitted.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, WorkflowEvent::AssistantText { .. }))
                .count(),
            300
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, WorkflowEvent::Failed { .. }))
        );
    }

    #[test]
    fn shutdown_cancels_and_cleans_every_active_job() {
        let decisions = Arc::new(Mutex::new(Vec::new()));
        let cancelled = Arc::new(Mutex::new(false));
        let cleanup_calls = Arc::new(Mutex::new(0));
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let mut controller = WorkflowController::new(
            Factory {
                events: Some(VecDeque::new()),
                decisions: decisions.clone(),
                cancelled: cancelled.clone(),
                create_error: None,
                submit_error: None,
                decide_error: None,
                event_error: None,
                cleanup_calls: cleanup_calls.clone(),
            },
            Sink(emitted.clone()),
        );
        let first = start(&mut controller);
        let second = start(&mut controller);

        controller.shutdown().unwrap();
        controller.shutdown().unwrap();

        assert!(*cancelled.lock().unwrap());
        assert_eq!(*cleanup_calls.lock().unwrap(), 2);
        assert_eq!(controller.poll(&first), Err(WorkflowError::TerminalJob));
        assert_eq!(controller.poll(&second), Err(WorkflowError::TerminalJob));
        assert_eq!(
            emitted
                .lock()
                .unwrap()
                .iter()
                .filter(|event| matches!(
                    event,
                    WorkflowEvent::StatusChanged {
                        status: JobStatus::Cancelled,
                        ..
                    }
                ))
                .count(),
            2
        );
        assert_eq!(
            decisions
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.ends_with(":Cancelled"))
                .count(),
            2
        );
    }

    #[test]
    fn reserved_start_honors_cancellation_before_factory_creation() {
        let (mut controller, _, _, emitted) = controller(vec![]);
        let cancellation = JobCancellation::default();
        cancellation.cancel();
        let job_id = controller.reserve_job_id();

        let receipt = controller
            .start_reserved(
                job_id.clone(),
                "input.docx".into(),
                "rewrite".into(),
                &cancellation,
            )
            .unwrap();

        assert_eq!(receipt.job_id, job_id);
        assert!(controller.is_terminal_job(&receipt.job_id));
        assert!(emitted.lock().unwrap().iter().any(|event| matches!(
            event,
            WorkflowEvent::StatusChanged {
                status: JobStatus::Cancelled,
                ..
            }
        )));
    }
}
