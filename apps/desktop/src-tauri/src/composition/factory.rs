use super::{
    CompositeJobCleanup, DynJobCleanup, DynModelSession, DynMutationAuthorization, JobSecrets,
    JobSecretsGenerator, JobWorkspace, JobWorkspaceFactory,
};
use ollama_cowork_core::{JobCleanup, WorkflowJob, WorkflowJobFactory};

pub struct RuntimeProvisioningRequest<'a> {
    pub job_id: &'a str,
    pub workspace: &'a JobWorkspace,
    pub secrets: &'a JobSecrets,
}

pub struct ProvisionedRuntime {
    pub session: DynModelSession,
    pub authorization: DynMutationAuthorization,
    pub cleanup: DynJobCleanup,
}

/// Provisions model and broker resources atomically or cleans up before failing.
pub trait RuntimeProvisioner: Send {
    fn provision(
        &mut self,
        request: RuntimeProvisioningRequest<'_>,
    ) -> Result<ProvisionedRuntime, String>;
}

pub struct DesktopWorkflowJobFactory<W, G, R> {
    workspaces: W,
    secrets: G,
    runtimes: R,
}

impl<W, G, R> DesktopWorkflowJobFactory<W, G, R> {
    pub fn new(workspaces: W, secrets: G, runtimes: R) -> Self {
        Self {
            workspaces,
            secrets,
            runtimes,
        }
    }
}

impl<W, G, R> WorkflowJobFactory for DesktopWorkflowJobFactory<W, G, R>
where
    W: JobWorkspaceFactory,
    G: JobSecretsGenerator,
    R: RuntimeProvisioner,
{
    type Session = DynModelSession;
    type Authorization = DynMutationAuthorization;
    type Cleanup = CompositeJobCleanup;

    fn create(
        &mut self,
        job_id: &str,
        source: &std::path::Path,
    ) -> Result<WorkflowJob<Self::Session, Self::Authorization, Self::Cleanup>, String> {
        let mut workspace = self.workspaces.create(job_id, source)?;
        let secrets = match self.secrets.generate().and_then(|value| {
            value.validate()?;
            Ok(value)
        }) {
            Ok(value) => value,
            Err(error) => {
                return Err(rollback_workspace(&mut workspace, error));
            }
        };
        let runtime = match self.runtimes.provision(RuntimeProvisioningRequest {
            job_id,
            workspace: &workspace,
            secrets: &secrets,
        }) {
            Ok(value) => value,
            Err(error) => {
                return Err(rollback_workspace(&mut workspace, error));
            }
        };
        Ok(WorkflowJob {
            session: runtime.session,
            authorization: runtime.authorization,
            cleanup: CompositeJobCleanup::new(vec![Box::new(workspace), Box::new(runtime.cleanup)]),
        })
    }
}

fn rollback_workspace(workspace: &mut JobWorkspace, error: String) -> String {
    match workspace.terminate() {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; workspace rollback failed: {cleanup}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ollama_cowork_core::{ModelEvent, ModelSession, MutationAuthorization, MutationDecision};
    use std::{
        fs,
        path::Path,
        sync::{Arc, Mutex},
    };

    struct Secrets(bool);
    impl JobSecretsGenerator for Secrets {
        fn generate(&mut self) -> Result<JobSecrets, String> {
            if self.0 {
                return Err("secret generation failed".into());
            }
            Ok(JobSecrets {
                opencode_password: "11111111111111111111111111111111".into(),
                broker_execution_token: "22222222222222222222222222222222".into(),
                broker_control_token: "33333333333333333333333333333333".into(),
                broker_job_token: "44444444444444444444444444444444".into(),
            })
        }
    }
    struct Session;
    impl ModelSession for Session {
        fn submit(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn decide(&mut self, _: &str, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn cancel(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn next_event(&mut self) -> Result<Option<ModelEvent>, String> {
            Ok(None)
        }
    }
    struct Authorization;
    impl MutationAuthorization for Authorization {
        fn decide(&mut self, _: &str, _: MutationDecision) -> Result<(), String> {
            Ok(())
        }
        fn revoke_unconsumed(&mut self, _: &str) -> Result<(), String> {
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
    struct Runtime {
        fail: bool,
        cleanup_calls: Arc<Mutex<usize>>,
    }
    impl RuntimeProvisioner for Runtime {
        fn provision(
            &mut self,
            request: RuntimeProvisioningRequest<'_>,
        ) -> Result<ProvisionedRuntime, String> {
            assert!(request.workspace.model().is_dir());
            assert_eq!(
                request.secrets.broker_job_token,
                "44444444444444444444444444444444"
            );
            if self.fail {
                return Err("runtime failed".into());
            }
            Ok(ProvisionedRuntime {
                session: DynModelSession::new(Session),
                authorization: DynMutationAuthorization::new(Authorization),
                cleanup: DynJobCleanup::new(Cleanup(self.cleanup_calls.clone())),
            })
        }
    }

    fn source(root: &Path) -> std::path::PathBuf {
        let source = root.join("source.docx");
        fs::write(&source, b"fixture").unwrap();
        source
    }

    #[test]
    fn successful_factory_composes_independent_handles_and_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let jobs = temp.path().join("jobs");
        let calls = Arc::new(Mutex::new(0));
        let mut factory = DesktopWorkflowJobFactory::new(
            super::super::FilesystemJobWorkspaceFactory::new(jobs.clone()).unwrap(),
            Secrets(false),
            Runtime {
                fail: false,
                cleanup_calls: calls.clone(),
            },
        );
        let mut job = factory.create("job-1", &source(temp.path())).unwrap();
        job.cleanup.terminate().unwrap();
        assert_eq!(*calls.lock().unwrap(), 1);
        assert!(!job_root_exists(&jobs, "job-1"));
    }

    #[test]
    fn secret_or_runtime_failure_rolls_back_workspace() {
        for (secret_failure, runtime_failure) in [(true, false), (false, true)] {
            let temp = tempfile::tempdir().unwrap();
            let jobs = temp.path().join("jobs");
            let mut factory = DesktopWorkflowJobFactory::new(
                super::super::FilesystemJobWorkspaceFactory::new(jobs.clone()).unwrap(),
                Secrets(secret_failure),
                Runtime {
                    fail: runtime_failure,
                    cleanup_calls: Arc::new(Mutex::new(0)),
                },
            );
            assert!(factory.create("job-1", &source(temp.path())).is_err());
            assert!(!job_root_exists(&jobs, "job-1"));
        }
    }

    fn job_root_exists(runs_root: &Path, job_id: &str) -> bool {
        fs::read_dir(runs_root)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.path().join(job_id).exists())
    }
}
