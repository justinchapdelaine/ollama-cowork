use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core::error::AppResult;

#[async_trait]
pub trait ApprovalReviewer: Send + Sync {
    async fn review(&self, request: ApprovalRequest) -> AppResult<ApprovalDecision>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub summary: String,
    pub requested_capability: RequestedCapability,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedCapability {
    Network,
    Install,
    DestructiveFilesystem,
    HostMutation,
    SandboxEscape,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecision {
    pub approved: bool,
    pub reviewer: String,
    pub reason: String,
}
