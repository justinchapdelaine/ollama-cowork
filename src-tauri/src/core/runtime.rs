use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::core::approval::{ApprovalRequest, ApprovalSubject, RequestedCapability};
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

impl CommandSpec {
    pub fn requested_capabilities(&self) -> Vec<RequestedCapability> {
        let mut capabilities = vec![RequestedCapability::Command];

        if matches!(self.network, NetworkPolicy::ApprovedOnline) {
            capabilities.push(RequestedCapability::NetworkAccess);
        }

        if is_likely_install_command(&self.program, &self.args) {
            capabilities.push(RequestedCapability::Install);
        }

        capabilities
    }

    pub fn approval_request(&self) -> ApprovalRequest {
        ApprovalRequest {
            id: Uuid::new_v4(),
            summary: format!("Run command `{}`", self.display_command()),
            subject: ApprovalSubject::RuntimeCommand {
                program: self.program.clone(),
                args: self.args.clone(),
                cwd: self.cwd.display(),
            },
            requested_capabilities: self.requested_capabilities(),
            reason: "runtime command execution is side-effecting and disabled until approved"
                .to_string(),
        }
    }

    fn display_command(&self) -> String {
        if self.args.is_empty() {
            return self.program.clone();
        }

        format!("{} {}", self.program, self.args.join(" "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    Offline,
    ApprovedOnline,
}

impl RelativePath {
    pub fn display(&self) -> String {
        self.0.display().to_string()
    }
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

fn is_likely_install_command(program: &str, args: &[String]) -> bool {
    let program = program.to_ascii_lowercase();
    if matches!(
        program.as_str(),
        "npm"
            | "npm.cmd"
            | "pnpm"
            | "pnpm.cmd"
            | "yarn"
            | "yarn.cmd"
            | "cargo"
            | "winget"
            | "choco"
            | "scoop"
            | "pip"
            | "pip3"
    ) {
        return args.iter().any(|arg| {
            matches!(
                arg.to_ascii_lowercase().as_str(),
                "install" | "add" | "update" | "upgrade"
            )
        });
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::approval::ApprovalPolicy;
    use crate::core::approval::{ApprovalRequirement, DefaultApprovalPolicy};

    #[test]
    fn command_specs_request_command_capability_by_default() {
        let command = command_spec("cargo", ["test"], NetworkPolicy::Offline);

        assert_eq!(
            command.requested_capabilities(),
            vec![RequestedCapability::Command]
        );
        assert_eq!(
            DefaultApprovalPolicy.evaluate(&command.approval_request()),
            ApprovalRequirement::RequireManualApproval
        );
    }

    #[test]
    fn online_commands_request_network_access() {
        let command = command_spec("git", ["fetch"], NetworkPolicy::ApprovedOnline);

        assert_eq!(
            command.requested_capabilities(),
            vec![
                RequestedCapability::Command,
                RequestedCapability::NetworkAccess
            ]
        );
    }

    #[test]
    fn install_commands_request_install_capability() {
        let command = command_spec("npm.cmd", ["install"], NetworkPolicy::ApprovedOnline);

        assert_eq!(
            command.requested_capabilities(),
            vec![
                RequestedCapability::Command,
                RequestedCapability::NetworkAccess,
                RequestedCapability::Install
            ]
        );
    }

    fn command_spec<const N: usize>(
        program: &str,
        args: [&str; N],
        network: NetworkPolicy,
    ) -> CommandSpec {
        CommandSpec {
            program: program.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            cwd: RelativePath(PathBuf::from(".")),
            timeout_ms: 30_000,
            network,
        }
    }
}
