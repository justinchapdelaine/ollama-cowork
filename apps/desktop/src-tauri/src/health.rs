use crate::{
    config::DesktopConfig,
    contracts::{ComponentHealth, ComponentState, DesktopHealth},
    opencode_identity, srt_identity,
};
use ollama_cowork_opencode_client::{ProcessError, require_version};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const EXPECTED_BRIDGE: &[u8] = include_bytes!("../../../../scripts/runtime/srt-docx-bridge.mjs");

pub trait DesktopPrerequisiteProbe: Send + Sync {
    fn runtime_tools(&self, config: &DesktopConfig) -> Result<String, String>;
    fn sandbox_helper(&self, config: &DesktopConfig) -> Result<String, String>;
}

#[derive(Default)]
pub struct SystemDesktopPrerequisiteProbe;

enum VersionIdentity<'a> {
    Exact(&'a str),
    Prefix(&'a str),
}

impl DesktopPrerequisiteProbe for SystemDesktopPrerequisiteProbe {
    fn runtime_tools(&self, config: &DesktopConfig) -> Result<String, String> {
        let settings = config.runtime_settings();
        for (name, path) in settings
            .required_files()
            .into_iter()
            .filter(|(name, _)| !matches!(*name, "opencode" | "SRT helper"))
        {
            require_file(name, path)?;
        }
        let node = executable_version(
            &config.node_executable,
            "Node.js",
            VersionIdentity::Prefix("v"),
        )?;
        let expected_broker = format!("ollama-cowork-broker-host {}", env!("CARGO_PKG_VERSION"));
        let broker = executable_version(
            &config.broker_host_executable,
            "broker host",
            VersionIdentity::Exact(&expected_broker),
        )?;
        let expected_docx = format!("ollama-cowork-docx-tool {}", env!("CARGO_PKG_VERSION"));
        let docx = executable_version(
            &config.docx_tool,
            "DOCX tool",
            VersionIdentity::Exact(&expected_docx),
        )?;
        verify_bridge(&config.srt_bridge)?;
        Ok(format!(
            "Verified Node.js {node}, {broker}, {docx}, and the pinned SRT bridge."
        ))
    }

    fn sandbox_helper(&self, config: &DesktopConfig) -> Result<String, String> {
        let helper = executable_version(
            &config.srt_win,
            "SRT helper",
            VersionIdentity::Exact(srt_identity::HELPER_VERSION),
        )?;
        match srt_identity::matches(&config.srt_win)? {
            true => Ok(helper),
            false => Err("SRT helper does not match the proof-tested package identity.".into()),
        }
    }
}

pub fn collect_with(config: &DesktopConfig, probe: &dyn DesktopPrerequisiteProbe) -> DesktopHealth {
    let opencode = opencode(config);
    let sandbox = sandbox(config, probe);
    let runtime_tools = runtime_tools(config, probe);
    let model_endpoint = ComponentHealth {
        state: ComponentState::Configured,
        version: Some(config.model.clone()),
        detail: format!(
            "Configured Ollama endpoint {}. Connectivity is checked before workflows.",
            config.ollama_origin
        ),
    };
    let ready = matches!(opencode.state, ComponentState::Ready)
        && matches!(sandbox.state, ComponentState::Ready)
        && matches!(runtime_tools.state, ComponentState::Ready)
        && matches!(model_endpoint.state, ComponentState::Configured);
    DesktopHealth {
        app_version: env!("CARGO_PKG_VERSION").into(),
        ready,
        opencode,
        sandbox,
        runtime_tools,
        model_endpoint,
    }
}

fn runtime_tools(config: &DesktopConfig, probe: &dyn DesktopPrerequisiteProbe) -> ComponentHealth {
    match probe.runtime_tools(config) {
        Ok(detail) => ComponentHealth {
            state: ComponentState::Ready,
            version: None,
            detail,
        },
        Err(detail) => ComponentHealth {
            state: ComponentState::Unavailable,
            version: None,
            detail,
        },
    }
}

fn opencode(config: &DesktopConfig) -> ComponentHealth {
    if !config.opencode_executable.is_file() {
        return ComponentHealth {
            state: ComponentState::Unavailable,
            version: None,
            detail: format!(
                "Expected pre-installed opencode at {}.",
                config.opencode_executable.display()
            ),
        };
    }
    match require_version(&config.opencode_executable, &config.opencode_version) {
        Ok(found) => match opencode_identity::matches(&config.opencode_executable) {
            Ok(true) => ComponentHealth {
                state: ComponentState::Ready,
                version: Some(found),
                detail: "Pinned opencode version and executable identity are available.".into(),
            },
            Ok(false) => ComponentHealth {
                state: ComponentState::Unsupported,
                version: Some(found),
                detail: "The opencode version matches, but this executable build was not validated for Spike 001.".into(),
            },
            Err(detail) => ComponentHealth {
                state: ComponentState::Unavailable,
                version: Some(found),
                detail,
            },
        },
        Err(ProcessError::Version { found, .. }) => ComponentHealth {
            state: ComponentState::Unsupported,
            version: Some(found.clone()),
            detail: format!(
                "Expected opencode {}, found {found}.",
                config.opencode_version
            ),
        },
        Err(error) => ComponentHealth {
            state: ComponentState::Unavailable,
            version: None,
            detail: format!("Could not run opencode version check: {error}"),
        },
    }
}

fn sandbox(config: &DesktopConfig, probe: &dyn DesktopPrerequisiteProbe) -> ComponentHealth {
    match probe.sandbox_helper(config) {
        Ok(helper_version) => ComponentHealth {
            state: ComponentState::Ready,
            version: Some(format!(
                "package {}; {helper_version}",
                config.srt_version
            )),
            detail: "Pinned Windows SRT helper responds correctly. Enforcement is rechecked by workflow execution.".into(),
        },
        Err(detail) => ComponentHealth {
            state: ComponentState::Unavailable,
            version: None,
            detail,
        },
    }
}

fn require_file(name: &str, path: &Path) -> Result<(), String> {
    path.is_file()
        .then_some(())
        .ok_or_else(|| format!("{name} is unavailable at {}.", path.display()))
}

fn executable_version(
    path: &Path,
    name: &str,
    expected_identity: VersionIdentity<'_>,
) -> Result<String, String> {
    require_file(name, path)?;
    let mut command = Command::new(path);
    command.arg("--version");
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command
        .output()
        .map_err(|error| format!("Could not run {name} version check: {error}"))?;
    let found = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let identity_matches = match expected_identity {
        VersionIdentity::Exact(expected) => found == expected,
        VersionIdentity::Prefix(expected) => found.starts_with(expected),
    };
    if !output.status.success() || !identity_matches || found.len() > 256 {
        return Err(format!(
            "{name} did not return the expected version identity."
        ));
    }
    Ok(found)
}

fn verify_bridge(path: &Path) -> Result<(), String> {
    require_file("SRT bridge", path)?;
    let actual = fs::read(path).map_err(|error| format!("Could not read SRT bridge: {error}"))?;
    let actual = Sha256::digest(actual);
    let expected = Sha256::digest(EXPECTED_BRIDGE);
    (actual == expected)
        .then_some(())
        .ok_or_else(|| "SRT bridge does not match the pinned application version.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Probe {
        runtime: Result<String, String>,
        sandbox: Result<String, String>,
    }

    impl DesktopPrerequisiteProbe for Probe {
        fn runtime_tools(&self, _: &DesktopConfig) -> Result<String, String> {
            self.runtime.clone()
        }

        fn sandbox_helper(&self, _: &DesktopConfig) -> Result<String, String> {
            self.sandbox.clone()
        }
    }

    fn config() -> DesktopConfig {
        DesktopConfig {
            opencode_executable: PathBuf::from("missing-opencode"),
            opencode_version: "1".into(),
            broker_host_executable: PathBuf::from("broker"),
            node_executable: PathBuf::from("node"),
            srt_bridge: PathBuf::from("bridge"),
            docx_tool: PathBuf::from("docx"),
            srt_win: PathBuf::from("srt"),
            srt_version: "1".into(),
            ollama_origin: "http://127.0.0.1:11434".into(),
            model: "model".into(),
            runs_root: PathBuf::from("runs"),
        }
    }

    #[test]
    fn replaceable_probe_controls_runtime_and_sandbox_readiness() {
        let ready = collect_with(
            &config(),
            &Probe {
                runtime: Ok("runtime verified".into()),
                sandbox: Ok("srt-win 1".into()),
            },
        );
        assert!(matches!(ready.runtime_tools.state, ComponentState::Ready));
        assert!(matches!(ready.sandbox.state, ComponentState::Ready));

        let unavailable = collect_with(
            &config(),
            &Probe {
                runtime: Err("runtime invalid".into()),
                sandbox: Err("sandbox invalid".into()),
            },
        );
        assert!(matches!(
            unavailable.runtime_tools.state,
            ComponentState::Unavailable
        ));
        assert!(matches!(
            unavailable.sandbox.state,
            ComponentState::Unavailable
        ));
    }
}
