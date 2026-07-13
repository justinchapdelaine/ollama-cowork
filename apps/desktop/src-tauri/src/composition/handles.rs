use ollama_cowork_core::{
    BrokerOperation, JobCleanup, ModelEvent, ModelSession, MutationAuthorization, MutationDecision,
};

pub struct DynModelSession(Box<dyn ModelSession>);

impl DynModelSession {
    pub fn new(session: impl ModelSession + 'static) -> Self {
        Self(Box::new(session))
    }
}

impl ModelSession for DynModelSession {
    fn submit(&mut self, instruction: &str) -> Result<(), String> {
        self.0.submit(instruction)
    }

    fn decide(&mut self, external_id: &str, approved_once: bool) -> Result<(), String> {
        self.0.decide(external_id, approved_once)
    }

    fn cancel(&mut self) -> Result<(), String> {
        self.0.cancel()
    }

    fn next_event(&mut self) -> Result<Option<ModelEvent>, String> {
        self.0.next_event()
    }
}

pub struct DynMutationAuthorization(Box<dyn MutationAuthorization>);

impl DynMutationAuthorization {
    pub fn new(authorization: impl MutationAuthorization + 'static) -> Self {
        Self(Box::new(authorization))
    }
}

impl MutationAuthorization for DynMutationAuthorization {
    fn decide(
        &mut self,
        action_id: &str,
        operation: &BrokerOperation,
        decision: MutationDecision,
    ) -> Result<(), String> {
        self.0.decide(action_id, operation, decision)
    }

    fn revoke_unconsumed(&mut self, action_id: &str) -> Result<(), String> {
        self.0.revoke_unconsumed(action_id)
    }
}

pub struct DynJobCleanup(Box<dyn JobCleanup>);

impl DynJobCleanup {
    pub fn new(cleanup: impl JobCleanup + 'static) -> Self {
        Self(Box::new(cleanup))
    }
}

impl JobCleanup for DynJobCleanup {
    fn terminate(&mut self) -> Result<(), String> {
        self.0.terminate()
    }
}

/// Terminates resources in reverse construction order and attempts every step.
pub struct CompositeJobCleanup {
    resources: Vec<Box<dyn JobCleanup>>,
    result: Option<Result<(), String>>,
}

impl CompositeJobCleanup {
    pub fn new(resources: Vec<Box<dyn JobCleanup>>) -> Self {
        Self {
            resources,
            result: None,
        }
    }
}

impl JobCleanup for CompositeJobCleanup {
    fn terminate(&mut self) -> Result<(), String> {
        if let Some(result) = &self.result {
            return result.clone();
        }
        let errors = self
            .resources
            .iter_mut()
            .rev()
            .filter_map(|resource| resource.terminate().err())
            .collect::<Vec<_>>();
        let result = if errors.is_empty() {
            Ok(())
        } else {
            Err(format!("job cleanup failed: {}", errors.join("; ")))
        };
        self.result = Some(result.clone());
        result
    }
}

impl Drop for CompositeJobCleanup {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct RecordedCleanup {
        name: &'static str,
        calls: Arc<Mutex<Vec<&'static str>>>,
        fail: bool,
    }

    impl JobCleanup for RecordedCleanup {
        fn terminate(&mut self) -> Result<(), String> {
            self.calls.lock().unwrap().push(self.name);
            self.fail
                .then(|| format!("{} failed", self.name))
                .map_or(Ok(()), Err)
        }
    }

    #[test]
    fn composite_cleanup_is_reverse_ordered_exhaustive_and_idempotent() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let resources = [("workspace", false), ("broker", true), ("opencode", false)]
            .into_iter()
            .map(|(name, fail)| {
                Box::new(RecordedCleanup {
                    name,
                    calls: calls.clone(),
                    fail,
                }) as Box<dyn JobCleanup>
            })
            .collect();
        let mut cleanup = CompositeJobCleanup::new(resources);

        assert_eq!(
            cleanup.terminate(),
            Err("job cleanup failed: broker failed".into())
        );
        assert_eq!(
            cleanup.terminate(),
            Err("job cleanup failed: broker failed".into())
        );
        assert_eq!(*calls.lock().unwrap(), ["opencode", "broker", "workspace"]);
    }
}
