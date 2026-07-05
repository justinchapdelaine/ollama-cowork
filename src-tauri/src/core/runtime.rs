use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::core::error::AppResult;

#[async_trait]
pub trait WorkspaceRuntime: Send + Sync {
    async fn prepare_workspace(&self, source_folder: &Path) -> AppResult<WorkspaceId>;
    async fn run_command(
        &self,
        workspace: WorkspaceId,
        command: CommandSpec,
    ) -> AppResult<CommandResult>;
    async fn read_file(&self, workspace: WorkspaceId, path: RelativePath) -> AppResult<Vec<u8>>;
    async fn write_file(
        &self,
        workspace: WorkspaceId,
        path: RelativePath,
        contents: Vec<u8>,
    ) -> AppResult<()>;
    async fn list_changes(&self, workspace: WorkspaceId) -> AppResult<Vec<WorkspaceChange>>;
    async fn export_patch(&self, workspace: WorkspaceId) -> AppResult<PatchBundle>;
    async fn destroy_workspace(&self, workspace: WorkspaceId) -> AppResult<()>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct WorkspaceId(pub Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelativePath(pub PathBuf);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: RelativePath,
    pub timeout_ms: u64,
    pub network: NetworkPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    Offline,
    ApprovedOnline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceChange {
    pub path: RelativePath,
    pub kind: WorkspaceChangeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: RelativePath },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchBundle {
    pub summary: String,
    pub patch: String,
    pub changes: Vec<WorkspaceChange>,
}
