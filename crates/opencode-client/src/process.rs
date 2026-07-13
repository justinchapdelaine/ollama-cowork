use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};
use thiserror::Error;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

#[derive(Clone, Debug)]
pub struct OpencodeProcessConfig {
    pub executable: PathBuf,
    pub expected_version: String,
    pub workspace: PathBuf,
    pub port: u16,
    pub environment: HashMap<String, String>,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("opencode process error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported opencode version: expected {expected}, found {found}")]
    Version { expected: String, found: String },
}

pub struct OpencodeProcess {
    child: Child,
}

pub fn require_version(executable: &Path, expected: &str) -> Result<String, ProcessError> {
    let mut command = Command::new(executable);
    configure_background_process(&mut command);
    let output = command.arg("--version").output()?;
    let found = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || found != expected {
        return Err(ProcessError::Version {
            expected: expected.into(),
            found,
        });
    }
    Ok(found)
}

impl OpencodeProcess {
    pub fn start(config: OpencodeProcessConfig) -> Result<Self, ProcessError> {
        require_version(&config.executable, &config.expected_version)?;
        let mut command = Command::new(config.executable);
        configure_background_process(&mut command);
        command
            .args([
                "serve",
                "--pure",
                "--hostname",
                "127.0.0.1",
                "--port",
                &config.port.to_string(),
            ])
            .current_dir(config.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .envs(config.environment);
        Ok(Self {
            child: command.spawn()?,
        })
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn stop(&mut self) -> Result<(), ProcessError> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
            self.child.wait()?;
        }
        Ok(())
    }
}
impl Drop for OpencodeProcess {
    fn drop(&mut self) {
        let _ = self.stop();
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
}
