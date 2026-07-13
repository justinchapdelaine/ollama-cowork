use crate::{BrokerError, SandboxLaunch, ToolExecution};
use std::path::{Path, PathBuf};

pub trait SandboxRunner {
    fn run(&self, launch: &SandboxLaunch) -> Result<ToolExecution, BrokerError>;
}
pub trait ArtifactPublisher {
    fn publish(
        &self,
        private_artifact: &Path,
        publish_directory: &Path,
    ) -> Result<PathBuf, BrokerError>;
}
