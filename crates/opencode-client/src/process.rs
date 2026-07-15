use ollama_cowork_core::JobCleanup;
use ollama_cowork_process_supervisor::ManagedChild;
use std::{
    collections::HashMap,
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};
use thiserror::Error;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const VERSION_PROBE_ATTEMPTS: usize = 3;
const VERSION_PROBE_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_VERSION_DIAGNOSTIC_CHARS: usize = 512;

fn configure_background_process(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
}

pub fn locate_executable(
    explicit: Option<PathBuf>,
    executable_name: &str,
    search_path: Option<OsString>,
    fallbacks: &[PathBuf],
) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    if let Some(path) = search_path {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join(executable_name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    fallbacks
        .iter()
        .find(|path| path.is_file())
        .or_else(|| fallbacks.first())
        .cloned()
        .unwrap_or_else(|| PathBuf::from(executable_name))
}

#[derive(Clone)]
pub struct OpencodeProcessConfig {
    pub executable: PathBuf,
    pub expected_version: String,
    pub workspace: PathBuf,
    pub port: u16,
    /// Prevent inherited user configuration and credentials from entering the
    /// managed server. Callers must provide every required environment value.
    pub clear_environment: bool,
    pub environment: HashMap<String, String>,
    pub stdout_log: Option<PathBuf>,
    pub stderr_log: Option<PathBuf>,
    pub log_limit_bytes: u64,
}

impl fmt::Debug for OpencodeProcessConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpencodeProcessConfig")
            .field("executable", &self.executable)
            .field("expected_version", &self.expected_version)
            .field("workspace", &self.workspace)
            .field("port", &self.port)
            .field("clear_environment", &self.clear_environment)
            .field("environment", &"[REDACTED]")
            .field("stdout_log", &self.stdout_log)
            .field("stderr_log", &self.stderr_log)
            .field("log_limit_bytes", &self.log_limit_bytes)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("opencode process error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported opencode version: expected {expected}, found {found}")]
    Version { expected: String, found: String },
}

pub struct OpencodeProcess {
    child: ManagedChild,
}

pub fn require_version(executable: &Path, expected: &str) -> Result<String, ProcessError> {
    let mut last_diagnostic = String::new();
    for attempt in 0..VERSION_PROBE_ATTEMPTS {
        let mut command = Command::new(executable);
        configure_background_process(&mut command);
        let output = command.arg("--version").output()?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if output.status.success() && stdout == expected {
            return Ok(stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        last_diagnostic = bounded_version_diagnostic(&stdout, &stderr);
        if attempt + 1 < VERSION_PROBE_ATTEMPTS {
            thread::sleep(VERSION_PROBE_RETRY_DELAY);
        }
    }
    Err(ProcessError::Version {
        expected: expected.into(),
        found: last_diagnostic,
    })
}

fn bounded_version_diagnostic(stdout: &str, stderr: &str) -> String {
    let value = if stdout.is_empty() { stderr } else { stdout };
    value.chars().take(MAX_VERSION_DIAGNOSTIC_CHARS).collect()
}

impl OpencodeProcess {
    pub fn start(config: OpencodeProcessConfig) -> Result<Self, ProcessError> {
        require_version(&config.executable, &config.expected_version)?;
        let mut command = Command::new(config.executable);
        if config.clear_environment {
            command.env_clear();
        }
        command
            .args([
                "serve",
                "--pure",
                "--hostname",
                "127.0.0.1",
                "--port",
                &config.port.to_string(),
                "--print-logs",
                "--log-level",
                "ERROR",
            ])
            .current_dir(config.workspace)
            .stdin(Stdio::null())
            .envs(config.environment);
        let child = match (config.stdout_log.as_deref(), config.stderr_log.as_deref()) {
            (Some(stdout), Some(stderr)) => ManagedChild::spawn_with_bounded_logs(
                &mut command,
                stdout,
                stderr,
                config.log_limit_bytes,
            )?,
            (None, None) => {
                command.stdout(Stdio::null()).stderr(Stdio::null());
                ManagedChild::spawn(&mut command)?
            }
            _ => {
                return Err(ProcessError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "stdout and stderr logs must be configured together",
                )));
            }
        };
        Ok(Self { child })
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn stop(&mut self) -> Result<(), ProcessError> {
        self.child.stop().map_err(ProcessError::Io)
    }
}

impl Drop for OpencodeProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl JobCleanup for OpencodeProcess {
    fn terminate(&mut self) -> Result<(), String> {
        self.stop().map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_executable_wins() {
        let explicit = PathBuf::from("explicit-opencode.exe");
        assert_eq!(
            locate_executable(Some(explicit.clone()), "opencode.exe", None, &[]),
            explicit
        );
    }

    #[test]
    fn process_config_debug_redacts_the_child_environment() {
        let config = OpencodeProcessConfig {
            executable: "opencode.exe".into(),
            expected_version: "1.2.3".into(),
            workspace: "workspace".into(),
            port: 43123,
            clear_environment: true,
            environment: HashMap::from([("SECRET".into(), "sensitive-value".into())]),
            stdout_log: None,
            stderr_log: None,
            log_limit_bytes: 1024,
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("sensitive-value"));
    }

    #[test]
    fn locates_executable_on_path_before_fallback() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("opencode.exe");
        std::fs::write(&executable, b"").unwrap();
        let search_path = std::env::join_paths([root.path()]).unwrap();
        assert_eq!(
            locate_executable(
                None,
                "opencode.exe",
                Some(search_path),
                &[PathBuf::from("fallback.exe")]
            ),
            executable
        );
    }

    #[test]
    fn version_diagnostics_prefer_stdout_and_are_bounded() {
        assert_eq!(bounded_version_diagnostic("1.2.3", "ignored"), "1.2.3");
        assert_eq!(bounded_version_diagnostic("", "failure"), "failure");
        assert_eq!(
            bounded_version_diagnostic("", &"x".repeat(MAX_VERSION_DIAGNOSTIC_CHARS + 20)).len(),
            MAX_VERSION_DIAGNOSTIC_CHARS
        );
    }
}
