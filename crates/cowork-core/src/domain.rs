use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use subtle::ConstantTimeEq;
use thiserror::Error;

pub const BROKER_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum BrokerOperation {
    Inspect,
    RewriteSection {
        heading: String,
        replacement_paragraphs: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrokerRequest {
    pub schema_version: u32,
    pub job_id: String,
    pub token: String,
    pub source_sha256: String,
    #[serde(flatten)]
    pub operation: BrokerOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalState {
    Pending,
    ApprovedOnce,
    Consumed,
    Rejected,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct DocumentJob {
    pub id: String,
    token_sha256: [u8; 32],
    pub source: PathBuf,
    pub source_sha256: String,
    pub private_output_directory: PathBuf,
    pub publish_directory: PathBuf,
    pub approval: ApprovalState,
    pub approval_action_id: Option<String>,
    pub approved_operation: Option<BrokerOperation>,
}

impl DocumentJob {
    pub fn new(
        id: String,
        token: &str,
        source: PathBuf,
        source_sha256: String,
        private_output_directory: PathBuf,
        publish_directory: PathBuf,
    ) -> Self {
        Self {
            id,
            token_sha256: Sha256::digest(token.as_bytes()).into(),
            source,
            source_sha256,
            private_output_directory,
            publish_directory,
            approval: ApprovalState::Pending,
            approval_action_id: None,
            approved_operation: None,
        }
    }
    pub fn token_matches(&self, token: &str) -> bool {
        let candidate: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        bool::from(self.token_sha256.ct_eq(&candidate))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxLaunch {
    pub source: PathBuf,
    pub private_output: PathBuf,
    pub operation: BrokerOperation,
    pub timeout_ms: u64,
    pub stdout_limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolExecution {
    pub private_artifact: Option<PathBuf>,
    pub source_sha256_after: String,
    pub structured_result: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BrokerResult {
    pub schema_version: u32,
    pub job_id: String,
    pub artifact: Option<PathBuf>,
    pub result: String,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum BrokerError {
    #[error("unsupported schema version")]
    UnsupportedSchema,
    #[error("unknown job")]
    UnknownJob,
    #[error("invalid broker token")]
    InvalidToken,
    #[error("source document hash is stale")]
    StaleSource,
    #[error("mutation has not been approved once")]
    ApprovalRequired,
    #[error("approval was rejected")]
    Rejected,
    #[error("job was cancelled")]
    Cancelled,
    #[error("approval has already been consumed")]
    ApprovalConsumed,
    #[error("invalid structured operation: {0}")]
    InvalidOperation(&'static str),
    #[error("sandbox execution failed: {0}")]
    Sandbox(String),
    #[error("artifact validation or publication failed: {0}")]
    Publication(String),
    #[error("source changed during execution")]
    SourceChanged,
}
