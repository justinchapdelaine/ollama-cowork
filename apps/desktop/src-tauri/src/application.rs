#[cfg(test)]
use crate::selection::LocalDocxPathPolicy;
use crate::selection::{DocumentPathPolicy, DocumentSelections, SelectedDocument};
use ollama_cowork_core::{
    JobCancellation, WorkflowCommand, WorkflowController, WorkflowError, WorkflowEventSink,
    WorkflowJobFactory, WorkflowReceipt, validate_workflow_instruction,
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

pub trait WorkflowEngine: Send {
    fn reserve_job_id(&mut self) -> String;
    fn start_reserved(
        &mut self,
        job_id: String,
        source: PathBuf,
        instruction: String,
        cancellation: &JobCancellation,
    ) -> Result<WorkflowReceipt, WorkflowError>;
    fn execute(&mut self, command: WorkflowCommand) -> Result<WorkflowReceipt, WorkflowError>;
    fn poll(&mut self, job_id: &str) -> Result<(), WorkflowError>;
    fn is_terminal_job(&self, job_id: &str) -> bool;
    fn shutdown(&mut self) -> Result<(), WorkflowError>;
}

impl<F, E> WorkflowEngine for WorkflowController<F, E>
where
    F: WorkflowJobFactory,
    E: WorkflowEventSink,
{
    fn reserve_job_id(&mut self) -> String {
        self.reserve_job_id()
    }

    fn start_reserved(
        &mut self,
        job_id: String,
        source: PathBuf,
        instruction: String,
        cancellation: &JobCancellation,
    ) -> Result<WorkflowReceipt, WorkflowError> {
        self.start_reserved(job_id, source, instruction, cancellation)
    }

    fn execute(&mut self, command: WorkflowCommand) -> Result<WorkflowReceipt, WorkflowError> {
        self.handle(command)
    }

    fn poll(&mut self, job_id: &str) -> Result<(), WorkflowError> {
        self.poll(job_id)
    }

    fn is_terminal_job(&self, job_id: &str) -> bool {
        self.is_terminal_job(job_id)
    }

    fn shutdown(&mut self) -> Result<(), WorkflowError> {
        self.shutdown()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationError {
    pub code: &'static str,
    pub message: &'static str,
}

impl ApplicationError {
    pub fn state_unavailable() -> Self {
        Self {
            code: "workflow_state_unavailable",
            message: "The workflow state is unavailable.",
        }
    }

    fn invalid_selection() -> Self {
        Self {
            code: "invalid_document_selection",
            message: "Select the DOCX again before starting the workflow.",
        }
    }

    fn busy() -> Self {
        Self {
            code: "workflow_busy",
            message: "Only one DOCX workflow can run at a time.",
        }
    }

    fn starting() -> Self {
        Self {
            code: "workflow_starting",
            message: "The workflow is still starting.",
        }
    }
}

impl From<WorkflowError> for ApplicationError {
    fn from(error: WorkflowError) -> Self {
        match error {
            WorkflowError::InvalidRequest(_) => Self {
                code: "invalid_request",
                message: "The workflow request is invalid.",
            },
            WorkflowError::UnknownJob => Self {
                code: "unknown_job",
                message: "The workflow job does not exist.",
            },
            WorkflowError::TerminalJob => Self {
                code: "terminal_job",
                message: "The workflow job has already finished.",
            },
            WorkflowError::ActionMismatch => Self {
                code: "action_mismatch",
                message: "The approval action does not match the pending request.",
            },
            WorkflowError::Model(_) => Self {
                code: "workflow_integration_failed",
                message: "The document workflow integration failed.",
            },
        }
    }
}

enum Lifecycle {
    Idle,
    Starting {
        job_id: String,
        cancellation: JobCancellation,
    },
    Active {
        job_id: String,
    },
}

pub struct WorkflowApplication {
    engine: Arc<Mutex<Box<dyn WorkflowEngine>>>,
    lifecycle: Mutex<Lifecycle>,
    selections: DocumentSelections,
}

impl WorkflowApplication {
    #[cfg(test)]
    pub fn new(engine: impl WorkflowEngine + 'static) -> Self {
        Self::with_path_policy(engine, Arc::new(LocalDocxPathPolicy))
    }

    pub fn with_path_policy(
        engine: impl WorkflowEngine + 'static,
        path_policy: Arc<dyn DocumentPathPolicy>,
    ) -> Self {
        Self {
            engine: Arc::new(Mutex::new(Box::new(engine))),
            lifecycle: Mutex::new(Lifecycle::Idle),
            selections: DocumentSelections::new(path_policy),
        }
    }

    pub fn register_selected_document(
        &self,
        path: &Path,
    ) -> Result<SelectedDocument, ApplicationError> {
        self.selections
            .register(path)
            .map_err(|_| ApplicationError::invalid_selection())
    }

    pub fn start_docx(
        self: &Arc<Self>,
        selection_id: String,
        instruction: String,
    ) -> Result<WorkflowReceipt, ApplicationError> {
        validate_workflow_instruction(&instruction)
            .map_err(WorkflowError::InvalidRequest)
            .map_err(ApplicationError::from)?;
        let mut lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| ApplicationError::state_unavailable())?;
        if !matches!(*lifecycle, Lifecycle::Idle) {
            return Err(ApplicationError::busy());
        }
        let source = self
            .selections
            .consume(&selection_id)
            .map_err(|_| ApplicationError::invalid_selection())?;
        let job_id = self.lock_engine()?.reserve_job_id();
        let cancellation = JobCancellation::default();
        *lifecycle = Lifecycle::Starting {
            job_id: job_id.clone(),
            cancellation: cancellation.clone(),
        };
        drop(lifecycle);

        let application = Arc::clone(self);
        let worker_job_id = job_id.clone();
        thread::Builder::new()
            .name(format!("docx-workflow-{job_id}"))
            .spawn(move || {
                let result = application
                    .lock_engine()
                    .and_then(|mut engine| {
                        let receipt = engine
                            .start_reserved(
                                worker_job_id.clone(),
                                source,
                                instruction,
                                &cancellation,
                            )
                            .map_err(ApplicationError::from)?;
                        if cancellation.is_cancelled()
                            && !engine.is_terminal_job(&worker_job_id)
                        {
                            engine
                                .execute(WorkflowCommand::Cancel {
                                    job_id: worker_job_id.clone(),
                                })
                                .map_err(ApplicationError::from)?;
                        }
                        Ok(receipt)
                    });
                if let Ok(mut lifecycle) = application.lifecycle.lock()
                    && matches!(&*lifecycle, Lifecycle::Starting { job_id, .. } if job_id == &worker_job_id)
                {
                    *lifecycle = if result.is_ok() && !cancellation.is_cancelled() {
                        Lifecycle::Active {
                            job_id: worker_job_id,
                        }
                    } else {
                        Lifecycle::Idle
                    };
                }
            })
            .map_err(|_| {
                if let Ok(mut lifecycle) = self.lifecycle.lock() {
                    *lifecycle = Lifecycle::Idle;
                }
                ApplicationError::state_unavailable()
            })?;

        Ok(WorkflowReceipt { job_id })
    }

    pub fn approve_once(
        &self,
        job_id: String,
        action_id: String,
    ) -> Result<WorkflowReceipt, ApplicationError> {
        self.require_active(&job_id)?;
        self.execute(WorkflowCommand::ApproveOnce { job_id, action_id })
    }

    pub fn reject(
        &self,
        job_id: String,
        action_id: String,
    ) -> Result<WorkflowReceipt, ApplicationError> {
        self.require_active(&job_id)?;
        let receipt = self.execute(WorkflowCommand::Reject { job_id, action_id })?;
        self.mark_idle(&receipt.job_id);
        Ok(receipt)
    }

    pub fn cancel(&self, job_id: String) -> Result<WorkflowReceipt, ApplicationError> {
        {
            let lifecycle = self
                .lifecycle
                .lock()
                .map_err(|_| ApplicationError::state_unavailable())?;
            match &*lifecycle {
                Lifecycle::Starting {
                    job_id: active,
                    cancellation,
                } if active == &job_id => {
                    cancellation.cancel();
                    return Ok(WorkflowReceipt { job_id });
                }
                Lifecycle::Starting { .. } => return Err(ApplicationError::busy()),
                Lifecycle::Active { job_id: active } if active == &job_id => {}
                Lifecycle::Active { .. } | Lifecycle::Idle => {
                    return Err(ApplicationError::from(WorkflowError::UnknownJob));
                }
            }
        }
        let receipt = self.execute(WorkflowCommand::Cancel { job_id })?;
        self.mark_idle(&receipt.job_id);
        Ok(receipt)
    }

    pub fn poll(&self, job_id: &str) -> Result<(), ApplicationError> {
        self.require_active(job_id)?;
        let mut engine = self.lock_engine()?;
        engine.poll(job_id).map_err(ApplicationError::from)?;
        let terminal = engine.is_terminal_job(job_id);
        drop(engine);
        if terminal {
            self.mark_idle(job_id);
        }
        Ok(())
    }

    pub fn shutdown(&self) -> Result<(), ApplicationError> {
        if let Ok(lifecycle) = self.lifecycle.lock()
            && let Lifecycle::Starting { cancellation, .. } = &*lifecycle
        {
            cancellation.cancel();
        }
        let result = self.lock_engine()?.shutdown().map_err(Into::into);
        if let Ok(mut lifecycle) = self.lifecycle.lock() {
            *lifecycle = Lifecycle::Idle;
        }
        result
    }

    fn execute(&self, command: WorkflowCommand) -> Result<WorkflowReceipt, ApplicationError> {
        self.lock_engine()?.execute(command).map_err(Into::into)
    }

    fn require_active(&self, job_id: &str) -> Result<(), ApplicationError> {
        let lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| ApplicationError::state_unavailable())?;
        match &*lifecycle {
            Lifecycle::Active { job_id: active } if active == job_id => Ok(()),
            Lifecycle::Starting { job_id: active, .. } if active == job_id => {
                Err(ApplicationError::starting())
            }
            _ => Err(ApplicationError::from(WorkflowError::UnknownJob)),
        }
    }

    fn mark_idle(&self, job_id: &str) {
        if let Ok(mut lifecycle) = self.lifecycle.lock()
            && matches!(&*lifecycle, Lifecycle::Active { job_id: active } if active == job_id)
        {
            *lifecycle = Lifecycle::Idle;
        }
    }

    fn lock_engine(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Box<dyn WorkflowEngine>>, ApplicationError> {
        self.engine
            .lock()
            .map_err(|_| ApplicationError::state_unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{Condvar, MutexGuard},
        time::Duration,
    };

    #[derive(Default)]
    struct Calls {
        commands: Vec<WorkflowCommand>,
        starts: Vec<String>,
        polls: Vec<String>,
        shutdowns: usize,
        next_job: usize,
        terminal: bool,
    }

    struct FakeEngine {
        calls: Arc<Mutex<Calls>>,
        start_gate: Option<Arc<(Mutex<bool>, Condvar)>>,
    }
    impl WorkflowEngine for FakeEngine {
        fn reserve_job_id(&mut self) -> String {
            let mut calls = self.calls.lock().unwrap();
            calls.next_job += 1;
            format!("job-{}", calls.next_job)
        }

        fn start_reserved(
            &mut self,
            job_id: String,
            _: PathBuf,
            _: String,
            cancellation: &JobCancellation,
        ) -> Result<WorkflowReceipt, WorkflowError> {
            self.calls.lock().unwrap().starts.push(job_id.clone());
            if let Some(gate) = &self.start_gate {
                let (lock, wake) = &**gate;
                let mut released = lock.lock().unwrap();
                while !*released && !cancellation.is_cancelled() {
                    let result = wake
                        .wait_timeout(released, Duration::from_millis(10))
                        .unwrap();
                    released = result.0;
                }
            }
            Ok(WorkflowReceipt { job_id })
        }

        fn execute(&mut self, command: WorkflowCommand) -> Result<WorkflowReceipt, WorkflowError> {
            let job_id = match &command {
                WorkflowCommand::ApproveOnce { job_id, .. }
                | WorkflowCommand::Reject { job_id, .. }
                | WorkflowCommand::Cancel { job_id } => job_id.clone(),
                WorkflowCommand::Start { .. } => unreachable!(),
            };
            self.calls.lock().unwrap().commands.push(command);
            Ok(WorkflowReceipt { job_id })
        }

        fn poll(&mut self, job_id: &str) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().polls.push(job_id.into());
            Ok(())
        }

        fn is_terminal_job(&self, _: &str) -> bool {
            self.calls.lock().unwrap().terminal
        }

        fn shutdown(&mut self) -> Result<(), WorkflowError> {
            self.calls.lock().unwrap().shutdowns += 1;
            Ok(())
        }
    }

    fn selected(app: &WorkflowApplication) -> (tempfile::TempDir, SelectedDocument) {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.docx");
        fs::write(&source, b"fixture").unwrap();
        let selected = app.register_selected_document(&source).unwrap();
        (root, selected)
    }

    fn wait_for_start(calls: &Arc<Mutex<Calls>>) -> MutexGuard<'_, Calls> {
        for _ in 0..100 {
            let guard = calls.lock().unwrap();
            if !guard.starts.is_empty() {
                return guard;
            }
            drop(guard);
            thread::sleep(Duration::from_millis(5));
        }
        panic!("workflow worker did not start")
    }

    #[test]
    fn start_uses_a_single_use_selection_and_returns_before_provisioning() {
        let calls = Arc::new(Mutex::new(Calls::default()));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let app = Arc::new(WorkflowApplication::new(FakeEngine {
            calls: calls.clone(),
            start_gate: Some(gate.clone()),
        }));
        let (_root, selected) = selected(&app);
        let receipt = app
            .start_docx(selected.selection_id.clone(), "rewrite".into())
            .unwrap();
        assert_eq!(receipt.job_id, "job-1");
        assert!(
            app.start_docx(selected.selection_id, "reuse".into())
                .is_err()
        );
        drop(wait_for_start(&calls));
        app.cancel(receipt.job_id).unwrap();
        let (lock, wake) = &*gate;
        *lock.lock().unwrap() = true;
        wake.notify_all();
        for _ in 0..100 {
            if calls
                .lock()
                .unwrap()
                .commands
                .iter()
                .any(|command| matches!(command, WorkflowCommand::Cancel { .. }))
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            calls
                .lock()
                .unwrap()
                .commands
                .iter()
                .any(|command| matches!(command, WorkflowCommand::Cancel { .. }))
        );
        app.shutdown().unwrap();
    }

    #[test]
    fn internal_errors_are_sanitized_for_the_frontend() {
        assert_eq!(
            ApplicationError::from(WorkflowError::Model("private child diagnostic")),
            ApplicationError {
                code: "workflow_integration_failed",
                message: "The document workflow integration failed.",
            }
        );
    }

    #[test]
    fn invalid_instruction_is_rejected_without_consuming_the_selection() {
        let calls = Arc::new(Mutex::new(Calls::default()));
        let app = Arc::new(WorkflowApplication::new(FakeEngine {
            calls,
            start_gate: None,
        }));
        let (_root, selected) = selected(&app);

        assert!(
            app.start_docx(selected.selection_id.clone(), "   ".into())
                .is_err()
        );
        assert!(
            app.start_docx(selected.selection_id, "rewrite".into())
                .is_ok()
        );
        app.shutdown().unwrap();
    }
}
