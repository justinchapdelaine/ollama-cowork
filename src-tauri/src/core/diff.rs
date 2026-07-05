use async_trait::async_trait;

use crate::core::error::AppResult;
use crate::core::runtime::PatchBundle;

#[async_trait]
pub trait DiffEngine: Send + Sync {
    async fn diff(&self, source: DiffSource) -> AppResult<PatchBundle>;
}

pub enum DiffSource {
    RuntimeWorkspace(crate::core::runtime::WorkspaceId),
}

#[async_trait]
pub trait PatchApplier: Send + Sync {
    async fn apply(&self, patch: PatchBundle) -> AppResult<ApplyResult>;
}

pub struct ApplyResult {
    pub applied_change_count: usize,
}
