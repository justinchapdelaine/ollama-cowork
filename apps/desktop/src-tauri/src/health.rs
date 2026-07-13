use crate::{
    config::DesktopConfig,
    contracts::{ComponentHealth, ComponentState, DesktopHealth},
};
use ollama_cowork_opencode_client::{ProcessError, require_version};

pub fn collect(config: &DesktopConfig) -> DesktopHealth {
    let opencode = opencode(config);
    let sandbox = sandbox(config);
    let model_endpoint = ComponentHealth {
        state: ComponentState::Configured,
        version: Some(config.model.clone()),
        detail: format!(
            "Configured private-LAN endpoint {}. Connectivity is checked before workflows.",
            config.ollama_origin
        ),
    };
    let ready = matches!(opencode.state, ComponentState::Ready)
        && matches!(sandbox.state, ComponentState::Ready)
        && matches!(model_endpoint.state, ComponentState::Ready);
    DesktopHealth {
        app_version: env!("CARGO_PKG_VERSION").into(),
        ready,
        opencode,
        sandbox,
        model_endpoint,
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
        Ok(found) => ComponentHealth {
            state: ComponentState::Ready,
            version: Some(found),
            detail: "Pinned opencode prerequisite is available.".into(),
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

fn sandbox(config: &DesktopConfig) -> ComponentHealth {
    if config.srt_win.is_file() {
        ComponentHealth{state:ComponentState::Ready,version:Some(config.srt_version.clone()),detail:"Pinned Windows SRT helper is installed. Enforcement is rechecked by workflow execution.".into()}
    } else {
        ComponentHealth {
            state: ComponentState::Unavailable,
            version: None,
            detail: format!(
                "Pinned SRT helper is missing at {}.",
                config.srt_win.display()
            ),
        }
    }
}
