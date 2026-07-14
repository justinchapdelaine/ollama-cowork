use crate::contracts::WORKFLOW_EVENT;
use ollama_cowork_core::{WorkflowEvent, WorkflowEventSink};
use tauri::{AppHandle, Emitter};

pub trait FrontendEventEmitter: Send + Sync {
    fn emit(&self, event: &WorkflowEvent) -> Result<(), String>;
}

#[derive(Clone)]
pub struct WorkflowEventBridge<T> {
    emitter: T,
}

impl<T> WorkflowEventBridge<T> {
    pub fn new(emitter: T) -> Self {
        Self { emitter }
    }
}

impl<T> WorkflowEventSink for WorkflowEventBridge<T>
where
    T: FrontendEventEmitter,
{
    fn emit(&self, event: WorkflowEvent) {
        // A closed WebView must not interrupt trusted runtime cleanup.
        let _ = self.emitter.emit(&event);
    }
}

#[derive(Clone)]
pub struct TauriFrontendEventEmitter {
    app: AppHandle,
}

impl TauriFrontendEventEmitter {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl FrontendEventEmitter for TauriFrontendEventEmitter {
    fn emit(&self, event: &WorkflowEvent) -> Result<(), String> {
        self.app
            .emit_to("main", WORKFLOW_EVENT, event)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ollama_cowork_core::JobStatus;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FakeEmitter(Arc<Mutex<Vec<WorkflowEvent>>>);
    impl FrontendEventEmitter for FakeEmitter {
        fn emit(&self, event: &WorkflowEvent) -> Result<(), String> {
            self.0.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    #[test]
    fn bridge_preserves_the_transport_neutral_event_contract() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let bridge = WorkflowEventBridge::new(FakeEmitter(events.clone()));
        bridge.emit(WorkflowEvent::StatusChanged {
            job_id: "job-1".into(),
            status: JobStatus::Running,
        });
        assert!(matches!(
            &events.lock().unwrap()[0],
            WorkflowEvent::StatusChanged { job_id, .. } if job_id == "job-1"
        ));
    }
}
