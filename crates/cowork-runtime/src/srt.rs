use ollama_cowork_core::{
    BrokerError, BrokerOperation, SandboxLaunch, SandboxRunner, ToolExecution,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, process::Command};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn configure_background_process(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
}

#[derive(Clone, Debug)]
pub struct SrtRunnerConfig {
    pub node: PathBuf,
    pub bridge: PathBuf,
    pub docx_tool: PathBuf,
    pub srt_win: PathBuf,
    pub read_roots: Vec<PathBuf>,
}
pub struct SrtRunner {
    config: SrtRunnerConfig,
}
impl SrtRunner {
    pub fn new(config: SrtRunnerConfig) -> Self {
        Self { config }
    }
}

#[derive(Serialize)]
struct BridgeRequest<'a> {
    schema_version: u32,
    node: &'a PathBuf,
    docx_tool: &'a PathBuf,
    srt_win: &'a PathBuf,
    read_roots: &'a [PathBuf],
    source: &'a PathBuf,
    output: &'a PathBuf,
    timeout_ms: u64,
    stdout_limit: usize,
    operation: &'a BrokerOperation,
    result_path: PathBuf,
}
#[derive(Deserialize)]
struct BridgeResult {
    passed: bool,
    source_sha256_after: String,
    private_artifact: Option<PathBuf>,
    tool_result: String,
    error: Option<String>,
}

impl SandboxRunner for SrtRunner {
    fn run(&self, launch: &SandboxLaunch) -> Result<ToolExecution, BrokerError> {
        let parent = launch
            .private_output
            .parent()
            .ok_or_else(|| BrokerError::Sandbox("private output has no parent".into()))?;
        let run = tempfile::Builder::new()
            .prefix("broker-srt-")
            .tempdir_in(parent)
            .map_err(|e| BrokerError::Sandbox(e.to_string()))?;
        let request_path = run.path().join("bridge-request.json");
        let result_path = run.path().join("bridge-result.json");
        let request = BridgeRequest {
            schema_version: 1,
            node: &self.config.node,
            docx_tool: &self.config.docx_tool,
            srt_win: &self.config.srt_win,
            read_roots: &self.config.read_roots,
            source: &launch.source,
            output: &launch.private_output,
            timeout_ms: launch.timeout_ms,
            stdout_limit: launch.stdout_limit,
            operation: &launch.operation,
            result_path: result_path.clone(),
        };
        fs::write(&request_path, serde_json::to_vec(&request).unwrap())
            .map_err(|e| BrokerError::Sandbox(e.to_string()))?;
        let mut command = Command::new(&self.config.node);
        configure_background_process(&mut command);
        let status = command
            .arg(&self.config.bridge)
            .arg(&request_path)
            .status()
            .map_err(|e| BrokerError::Sandbox(e.to_string()))?;
        let bytes = fs::read(&result_path)
            .map_err(|e| BrokerError::Sandbox(format!("bridge result missing: {e}")))?;
        let result: BridgeResult =
            serde_json::from_slice(&bytes).map_err(|e| BrokerError::Sandbox(e.to_string()))?;
        if !status.success() || !result.passed {
            return Err(BrokerError::Sandbox(
                result
                    .error
                    .unwrap_or_else(|| format!("SRT bridge exited {status}")),
            ));
        }
        Ok(ToolExecution {
            private_artifact: result.private_artifact,
            source_sha256_after: result.source_sha256_after,
            structured_result: result.tool_result,
        })
    }
}
