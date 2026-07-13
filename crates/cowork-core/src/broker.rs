use crate::{
    ApprovalState, ArtifactPublisher, BROKER_SCHEMA_VERSION, BrokerError, BrokerOperation,
    BrokerRequest, BrokerResult, DocumentJob, SandboxLaunch, SandboxRunner,
};
use std::{collections::HashMap, fs};

pub struct ToolBroker<R, P> {
    jobs: HashMap<String, DocumentJob>,
    runner: R,
    publisher: P,
}

impl<R: SandboxRunner, P: ArtifactPublisher> ToolBroker<R, P> {
    pub fn new(runner: R, publisher: P) -> Self {
        Self {
            jobs: HashMap::new(),
            runner,
            publisher,
        }
    }
    pub fn register(&mut self, mut job: DocumentJob) -> Result<(), BrokerError> {
        if self.jobs.contains_key(&job.id) {
            return Err(BrokerError::InvalidOperation("duplicate job id"));
        }
        if job.id.trim().is_empty() {
            return Err(BrokerError::InvalidOperation("empty job id"));
        }
        job.source = fs::canonicalize(&job.source)
            .map_err(|_| BrokerError::InvalidOperation("source is not canonicalizable"))?;
        job.private_output_directory = fs::canonicalize(&job.private_output_directory)
            .map_err(|_| BrokerError::InvalidOperation("private output is not canonicalizable"))?;
        job.publish_directory = fs::canonicalize(&job.publish_directory).map_err(|_| {
            BrokerError::InvalidOperation("publish directory is not canonicalizable")
        })?;
        if job
            .source
            .extension()
            .and_then(|v| v.to_str())
            .map(|v| v.eq_ignore_ascii_case("docx"))
            != Some(true)
        {
            return Err(BrokerError::InvalidOperation("source is not a DOCX"));
        }
        if job.private_output_directory == job.publish_directory {
            return Err(BrokerError::InvalidOperation(
                "private and publish directories must differ",
            ));
        }
        self.jobs.insert(job.id.clone(), job);
        Ok(())
    }
    pub fn decide(
        &mut self,
        job_id: &str,
        action_id: &str,
        operation: BrokerOperation,
        decision: ApprovalState,
    ) -> Result<(), BrokerError> {
        if !matches!(
            decision,
            ApprovalState::ApprovedOnce | ApprovalState::Rejected | ApprovalState::Cancelled
        ) {
            return Err(BrokerError::InvalidOperation(
                "invalid external approval decision",
            ));
        }
        if action_id.trim().is_empty() {
            return Err(BrokerError::InvalidOperation("empty approval action id"));
        }
        let job = self.jobs.get_mut(job_id).ok_or(BrokerError::UnknownJob)?;
        if !matches!(job.approval, ApprovalState::Pending) {
            return Err(BrokerError::InvalidOperation(
                "approval is already terminal",
            ));
        }
        job.approval_action_id = Some(action_id.into());
        job.approved_operation = Some(operation);
        job.approval = decision;
        Ok(())
    }

    pub fn revoke_unconsumed(&mut self, job_id: &str, action_id: &str) -> Result<(), BrokerError> {
        let job = self.jobs.get_mut(job_id).ok_or(BrokerError::UnknownJob)?;
        if job.approval_action_id.as_deref() != Some(action_id) {
            return Err(BrokerError::InvalidOperation(
                "approval action does not match",
            ));
        }
        match job.approval {
            ApprovalState::Pending | ApprovalState::ApprovedOnce => {
                job.approval = ApprovalState::Cancelled;
                Ok(())
            }
            ApprovalState::Consumed => Err(BrokerError::ApprovalConsumed),
            ApprovalState::Rejected | ApprovalState::Cancelled => Err(
                BrokerError::InvalidOperation("approval is already terminal"),
            ),
        }
    }
    pub fn execute(&mut self, request: BrokerRequest) -> Result<BrokerResult, BrokerError> {
        if request.schema_version != BROKER_SCHEMA_VERSION {
            return Err(BrokerError::UnsupportedSchema);
        }
        let job = self
            .jobs
            .get_mut(&request.job_id)
            .ok_or(BrokerError::UnknownJob)?;
        if !job.token_matches(&request.token) {
            return Err(BrokerError::InvalidToken);
        }
        if request.source_sha256 != job.source_sha256 {
            return Err(BrokerError::StaleSource);
        }
        validate_operation(&request.operation)?;
        let mutation = matches!(request.operation, BrokerOperation::RewriteSection { .. });
        if mutation {
            if job.approved_operation.as_ref() != Some(&request.operation) {
                return Err(BrokerError::ApprovalRequired);
            }
            match job.approval {
                ApprovalState::ApprovedOnce => job.approval = ApprovalState::Consumed,
                ApprovalState::Rejected => return Err(BrokerError::Rejected),
                ApprovalState::Cancelled => return Err(BrokerError::Cancelled),
                ApprovalState::Consumed => return Err(BrokerError::ApprovalConsumed),
                ApprovalState::Pending => return Err(BrokerError::ApprovalRequired),
            }
        }
        let private_output = job
            .private_output_directory
            .join(format!("{}.revised.docx", job.id));
        let launch = SandboxLaunch {
            source: job.source.clone(),
            private_output,
            operation: request.operation,
            timeout_ms: 30_000,
            stdout_limit: 1024 * 1024,
        };
        let execution = self.runner.run(&launch)?;
        if execution.source_sha256_after != job.source_sha256 {
            return Err(BrokerError::SourceChanged);
        }
        let artifact = match execution.private_artifact {
            Some(path) => {
                let actual = fs::canonicalize(&path)
                    .map_err(|_| BrokerError::Publication("private artifact is missing".into()))?;
                let expected = fs::canonicalize(&launch.private_output).map_err(|_| {
                    BrokerError::Publication("assigned private artifact was not created".into())
                })?;
                if actual != expected {
                    return Err(BrokerError::Publication(
                        "runner returned an unexpected artifact path".into(),
                    ));
                }
                Some(self.publisher.publish(&actual, &job.publish_directory)?)
            }
            None => None,
        };
        Ok(BrokerResult {
            schema_version: BROKER_SCHEMA_VERSION,
            job_id: job.id.clone(),
            artifact,
            result: execution.structured_result,
        })
    }
    pub fn approval(&self, job_id: &str) -> Option<&ApprovalState> {
        self.jobs.get(job_id).map(|j| &j.approval)
    }
}

fn validate_operation(operation: &BrokerOperation) -> Result<(), BrokerError> {
    if let BrokerOperation::RewriteSection {
        heading,
        replacement_paragraphs,
    } = operation
    {
        if heading.trim().is_empty() {
            return Err(BrokerError::InvalidOperation("empty heading"));
        }
        if replacement_paragraphs.is_empty() {
            return Err(BrokerError::InvalidOperation("empty replacement"));
        }
        if replacement_paragraphs.len() > 32
            || replacement_paragraphs
                .iter()
                .any(|p| p.len() > 16_384 || p.contains('\0'))
        {
            return Err(BrokerError::InvalidOperation("replacement exceeds limits"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolExecution;
    use std::{
        cell::Cell,
        path::{Path, PathBuf},
    };
    use tempfile::TempDir;
    struct Runner {
        calls: Cell<usize>,
        hash: String,
        artifact: Option<PathBuf>,
    }
    impl SandboxRunner for Runner {
        fn run(&self, _: &SandboxLaunch) -> Result<ToolExecution, BrokerError> {
            self.calls.set(self.calls.get() + 1);
            Ok(ToolExecution {
                private_artifact: self.artifact.clone(),
                source_sha256_after: self.hash.clone(),
                structured_result: "ok".into(),
            })
        }
    }
    struct Publisher;
    impl ArtifactPublisher for Publisher {
        fn publish(&self, p: &Path, d: &Path) -> Result<PathBuf, BrokerError> {
            Ok(d.join(p.file_name().unwrap()))
        }
    }
    fn broker() -> (ToolBroker<Runner, Publisher>, TempDir) {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.docx");
        std::fs::write(&source, b"fixture").unwrap();
        let private = root.path().join("private");
        let published = root.path().join("published");
        std::fs::create_dir(&private).unwrap();
        std::fs::create_dir(&published).unwrap();
        let artifact = private.join("job.revised.docx");
        std::fs::write(&artifact, b"artifact").unwrap();
        let runner = Runner {
            calls: Cell::new(0),
            hash: "abc".into(),
            artifact: Some(artifact),
        };
        let mut b = ToolBroker::new(runner, Publisher);
        b.register(DocumentJob::new(
            "job".into(),
            "secret",
            source,
            "abc".into(),
            private,
            published,
        ))
        .unwrap();
        (b, root)
    }
    fn rewrite(token: &str) -> BrokerRequest {
        BrokerRequest {
            schema_version: 1,
            job_id: "job".into(),
            token: token.into(),
            source_sha256: "abc".into(),
            operation: BrokerOperation::RewriteSection {
                heading: "Executive Summary".into(),
                replacement_paragraphs: vec!["new".into()],
            },
        }
    }
    #[test]
    fn denies_invalid_token_without_execution() {
        let (mut b, _root) = broker();
        assert_eq!(b.execute(rewrite("wrong")), Err(BrokerError::InvalidToken));
        assert_eq!(b.runner.calls.get(), 0);
    }
    #[test]
    fn denies_unapproved_mutation_without_execution() {
        let (mut b, _root) = broker();
        assert_eq!(
            b.execute(rewrite("secret")),
            Err(BrokerError::ApprovalRequired)
        );
        assert_eq!(b.runner.calls.get(), 0);
    }
    #[test]
    fn consumes_approval_before_execution() {
        let (mut b, _root) = broker();
        b.decide(
            "job",
            "action",
            rewrite("secret").operation,
            ApprovalState::ApprovedOnce,
        )
        .unwrap();
        assert!(b.execute(rewrite("secret")).is_ok());
        assert_eq!(b.approval("job"), Some(&ApprovalState::Consumed));
        assert_eq!(
            b.execute(rewrite("secret")),
            Err(BrokerError::ApprovalConsumed)
        );
        assert_eq!(b.runner.calls.get(), 1);
    }
    #[test]
    fn rejection_and_cancellation_do_not_execute() {
        for state in [ApprovalState::Rejected, ApprovalState::Cancelled] {
            let (mut b, _root) = broker();
            b.decide("job", "action", rewrite("secret").operation, state.clone())
                .unwrap();
            assert!(b.execute(rewrite("secret")).is_err());
            assert_eq!(b.runner.calls.get(), 0);
        }
    }
    #[test]
    fn stale_hash_does_not_execute() {
        let (mut b, _root) = broker();
        let mut r = rewrite("secret");
        r.source_sha256 = "old".into();
        assert_eq!(b.execute(r), Err(BrokerError::StaleSource));
        assert_eq!(b.runner.calls.get(), 0);
    }

    #[test]
    fn revocation_is_action_correlated_and_denies_execution() {
        let (mut b, _root) = broker();
        b.decide(
            "job",
            "expected-action",
            rewrite("secret").operation,
            ApprovalState::ApprovedOnce,
        )
        .unwrap();
        assert!(b.revoke_unconsumed("job", "wrong-action").is_err());
        b.revoke_unconsumed("job", "expected-action").unwrap();
        assert_eq!(b.execute(rewrite("secret")), Err(BrokerError::Cancelled));
        assert_eq!(b.runner.calls.get(), 0);
    }

    #[test]
    fn consumed_approval_cannot_be_revoked_or_reused() {
        let (mut b, _root) = broker();
        b.decide(
            "job",
            "action",
            rewrite("secret").operation,
            ApprovalState::ApprovedOnce,
        )
        .unwrap();
        assert!(b.execute(rewrite("secret")).is_ok());
        assert_eq!(
            b.revoke_unconsumed("job", "action"),
            Err(BrokerError::ApprovalConsumed)
        );
        assert_eq!(b.runner.calls.get(), 1);
    }

    #[test]
    fn approval_is_bound_to_the_exact_rewrite_operation() {
        let (mut b, _root) = broker();
        let approved = rewrite("secret");
        b.decide(
            "job",
            "action",
            approved.operation.clone(),
            ApprovalState::ApprovedOnce,
        )
        .unwrap();
        let mut changed = approved;
        changed.operation = BrokerOperation::RewriteSection {
            heading: "Executive Summary".into(),
            replacement_paragraphs: vec!["different text".into()],
        };
        assert_eq!(b.execute(changed), Err(BrokerError::ApprovalRequired));
        assert_eq!(b.runner.calls.get(), 0);
        assert_eq!(b.approval("job"), Some(&ApprovalState::ApprovedOnce));
    }
}
