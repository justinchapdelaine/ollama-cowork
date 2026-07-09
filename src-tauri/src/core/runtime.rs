use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command as StdCommand, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::core::approval::{ApprovalRequest, ApprovalSubject, RequestedCapability};
use crate::core::error::{AppError, AppResult};
use crate::core::session::SessionId;

const COMMAND_OUTPUT_LIMIT_BYTES: usize = 200_000;

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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}

#[async_trait]
pub trait RuntimeCommandRunner: Send + Sync {
    async fn run(&self, command: &QueuedRuntimeCommand) -> AppResult<CommandResult>;
}

#[derive(Debug, Clone)]
pub struct QueuedRuntimeCommand {
    pub request_id: Uuid,
    pub session_id: SessionId,
    pub run_id: Option<Uuid>,
    pub workspace_id: Uuid,
    pub workspace_root: PathBuf,
    pub resolved_cwd: PathBuf,
    pub command: CommandSpec,
}

#[derive(Debug, Default)]
pub struct RuntimeCommandQueue {
    pending: Mutex<HashMap<Uuid, QueuedRuntimeCommand>>,
}

impl RuntimeCommandQueue {
    pub fn insert(&self, command: QueuedRuntimeCommand) -> AppResult<()> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|err| AppError::Runtime(format!("runtime command queue poisoned: {err}")))?;

        if pending.contains_key(&command.request_id) {
            return Err(AppError::Runtime(format!(
                "runtime command already queued: {}",
                command.request_id
            )));
        }

        pending.insert(command.request_id, command);
        Ok(())
    }

    pub fn take(&self, request_id: Uuid) -> AppResult<Option<QueuedRuntimeCommand>> {
        Ok(self
            .pending
            .lock()
            .map_err(|err| AppError::Runtime(format!("runtime command queue poisoned: {err}")))?
            .remove(&request_id))
    }

    pub fn remove(&self, request_id: Uuid) -> AppResult<()> {
        self.pending
            .lock()
            .map_err(|err| AppError::Runtime(format!("runtime command queue poisoned: {err}")))?
            .remove(&request_id);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeCommandResolution {
    Completed {
        message_id: Uuid,
        request_id: Uuid,
        command: CommandSpec,
        result: CommandResult,
    },
    Failed {
        message_id: Uuid,
        request_id: Uuid,
        command: CommandSpec,
        message: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct HostCommandRunner;

#[async_trait]
impl RuntimeCommandRunner for HostCommandRunner {
    async fn run(&self, command: &QueuedRuntimeCommand) -> AppResult<CommandResult> {
        if command.command.program.trim().is_empty() {
            return Err(AppError::InvalidConfig(
                "runtime command program cannot be empty".to_string(),
            ));
        }

        if !command.resolved_cwd.is_dir() {
            return Err(AppError::InvalidConfig(format!(
                "runtime command cwd is not a directory: {}",
                command.command.cwd.display()
            )));
        }

        if !command.resolved_cwd.starts_with(&command.workspace_root) {
            return Err(AppError::PolicyDenied(format!(
                "runtime command cwd escapes selected workspace: {}",
                command.command.cwd.display()
            )));
        }

        let command = command.clone();
        tokio::task::spawn_blocking(move || run_command_blocking(command))
            .await
            .map_err(|err| AppError::Runtime(format!("runtime command worker failed: {err}")))?
    }
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

fn decode_output(output: &[u8]) -> String {
    let was_truncated = output.len() > COMMAND_OUTPUT_LIMIT_BYTES;
    let output = if output.len() > COMMAND_OUTPUT_LIMIT_BYTES {
        &output[..COMMAND_OUTPUT_LIMIT_BYTES]
    } else {
        output
    };
    let mut text = String::from_utf8_lossy(output).into_owned();

    if was_truncated {
        while !text.is_char_boundary(text.len()) {
            text.pop();
        }
        text.push_str("\n...[truncated]");
    }

    text
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn run_command_blocking(command: QueuedRuntimeCommand) -> AppResult<CommandResult> {
    let start = Instant::now();
    let timeout_after = Duration::from_millis(command.command.timeout_ms.max(1));
    let (output_files, stdout_file, stderr_file) = RuntimeOutputFiles::create(command.request_id)?;
    let mut child = StdCommand::new(&command.command.program)
        .args(&command.command.args)
        .current_dir(&command.resolved_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .map_err(|err| {
            AppError::Runtime(format!(
                "spawn runtime command `{}`: {err}",
                command.command.display_command()
            ))
        })?;

    loop {
        match child.try_wait().map_err(|err| {
            AppError::Runtime(format!(
                "poll runtime command `{}`: {err}",
                command.command.display_command()
            ))
        })? {
            Some(status) => {
                return Ok(CommandResult {
                    exit_code: status.code(),
                    stdout: output_files.read_stdout()?,
                    stderr: output_files.read_stderr()?,
                    duration_ms: elapsed_ms(start),
                    timed_out: false,
                });
            }
            None if start.elapsed() >= timeout_after => {
                let _ = child.kill();
                let status = child.wait().map_err(|err| {
                    AppError::Runtime(format!(
                        "wait for timed-out runtime command `{}`: {err}",
                        command.command.display_command()
                    ))
                })?;
                return Ok(CommandResult {
                    exit_code: status.code(),
                    stdout: output_files.read_stdout()?,
                    stderr: with_timeout_message(
                        output_files.read_stderr()?,
                        command.command.timeout_ms.max(1),
                    ),
                    duration_ms: elapsed_ms(start),
                    timed_out: true,
                });
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    }
}

struct RuntimeOutputFiles {
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

impl RuntimeOutputFiles {
    fn create(request_id: Uuid) -> AppResult<(Self, File, File)> {
        let root = std::env::temp_dir().join("ollama-cowork-runtime");
        fs::create_dir_all(&root)
            .map_err(|err| AppError::Runtime(format!("create runtime temp directory: {err}")))?;
        let stdout_path = root.join(format!("{request_id}-stdout.log"));
        let stderr_path = root.join(format!("{request_id}-stderr.log"));
        let stdout_file = File::create(&stdout_path)
            .map_err(|err| AppError::Runtime(format!("create runtime stdout capture: {err}")))?;
        let stderr_file = File::create(&stderr_path)
            .map_err(|err| AppError::Runtime(format!("create runtime stderr capture: {err}")))?;

        Ok((
            Self {
                stdout_path,
                stderr_path,
            },
            stdout_file,
            stderr_file,
        ))
    }

    fn read_stdout(&self) -> AppResult<String> {
        read_bounded_output(&self.stdout_path)
    }

    fn read_stderr(&self) -> AppResult<String> {
        read_bounded_output(&self.stderr_path)
    }
}

impl Drop for RuntimeOutputFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.stdout_path);
        let _ = fs::remove_file(&self.stderr_path);
    }
}

fn read_bounded_output(path: &Path) -> AppResult<String> {
    let mut file = File::open(path)
        .map_err(|err| AppError::Runtime(format!("open runtime output capture: {err}")))?;
    let mut output = Vec::new();
    file.by_ref()
        .take((COMMAND_OUTPUT_LIMIT_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|err| AppError::Runtime(format!("read runtime output capture: {err}")))?;
    Ok(decode_output(&output))
}

fn with_timeout_message(stderr: String, timeout_ms: u64) -> String {
    let message = format!("command timed out after {timeout_ms} ms");
    if stderr.trim().is_empty() {
        message
    } else {
        format!("{stderr}\n{message}")
    }
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

    #[test]
    fn runtime_command_queue_returns_queued_commands_once() {
        let queue = RuntimeCommandQueue::default();
        let request_id = Uuid::new_v4();
        let command = QueuedRuntimeCommand {
            request_id,
            session_id: SessionId(Uuid::new_v4()),
            run_id: None,
            workspace_id: Uuid::new_v4(),
            workspace_root: PathBuf::from("."),
            resolved_cwd: PathBuf::from("."),
            command: command_spec("cargo", ["test"], NetworkPolicy::Offline),
        };

        queue.insert(command).expect("insert command");

        assert!(queue.take(request_id).expect("take command").is_some());
        assert!(queue.take(request_id).expect("take again").is_none());
    }

    #[tokio::test]
    async fn host_command_runner_captures_command_output() {
        let workspace_root = std::env::current_dir().expect("current dir");
        let (program, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), "echo hello".to_string()],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "echo hello".to_string()],
            )
        };
        let command = QueuedRuntimeCommand {
            request_id: Uuid::new_v4(),
            session_id: SessionId(Uuid::new_v4()),
            run_id: None,
            workspace_id: Uuid::new_v4(),
            workspace_root: workspace_root.clone(),
            resolved_cwd: workspace_root,
            command: CommandSpec {
                program,
                args,
                cwd: RelativePath(PathBuf::from(".")),
                timeout_ms: 5_000,
                network: NetworkPolicy::Offline,
            },
        };

        let result = HostCommandRunner.run(&command).await.expect("run command");

        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("hello"));
        assert!(!result.timed_out);
    }
}
