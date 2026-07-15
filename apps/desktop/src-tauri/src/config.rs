use crate::composition::RuntimeSettings;
use crate::opencode_identity;
use crate::runtime_assets::{
    RuntimeAssetOverrides, RuntimeAssetResolver, SiblingRuntimeAssetResolver,
};
use crate::srt_identity;
use ollama_cowork_opencode_client::locate_executable;
use std::time::Duration;
use std::{env, path::PathBuf};

#[derive(Clone, Debug)]
pub struct DesktopConfig {
    pub opencode_executable: PathBuf,
    pub opencode_version: String,
    pub broker_host_executable: PathBuf,
    pub node_executable: PathBuf,
    pub srt_bridge: PathBuf,
    pub docx_tool: PathBuf,
    pub srt_win: PathBuf,
    pub srt_version: String,
    pub ollama_origin: String,
    pub model: String,
    pub runs_root: PathBuf,
}

impl DesktopConfig {
    pub fn load() -> Result<Self, String> {
        Self::load_with(&SiblingRuntimeAssetResolver)
    }

    fn load_with(resolver: &dyn RuntimeAssetResolver) -> Result<Self, String> {
        let fallbacks = env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("npm/node_modules/opencode-ai/bin/opencode.exe"))
            .into_iter()
            .collect::<Vec<_>>();
        let path_opencode = locate_executable(None, "opencode.exe", env::var_os("PATH"), &[]);
        let opencode_executable = opencode_identity::select(
            env::var_os("OLLAMA_COWORK_OPENCODE").map(PathBuf::from),
            path_opencode,
            fallbacks,
        );
        let node_executable = locate_executable(
            env::var_os("OLLAMA_COWORK_NODE").map(PathBuf::from),
            "node.exe",
            env::var_os("PATH"),
            &[],
        );
        let executable_directory = env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(PathBuf::from))
            .unwrap_or_default();
        let assets = resolver.resolve(
            &executable_directory,
            RuntimeAssetOverrides {
                broker_host: env::var_os("OLLAMA_COWORK_BROKER_HOST").map(PathBuf::from),
                docx_tool: env::var_os("OLLAMA_COWORK_DOCX_TOOL").map(PathBuf::from),
                srt_bridge: env::var_os("OLLAMA_COWORK_SRT_BRIDGE").map(PathBuf::from),
            },
        );
        let srt_win = env::var_os("OLLAMA_COWORK_SRT_WIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(r"C:\Program Files\ollama-cowork-spike\srt\0.0.65\srt-win.exe")
            });
        let ollama_origin = env::var("OLLAMA_COWORK_OLLAMA_ORIGIN")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".into());
        let model = env::var("OLLAMA_COWORK_OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:12b".into());
        let runs_root = env::var_os("OLLAMA_COWORK_RUNS_ROOT")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .map(|path| path.join("Ollama Cowork/runs"))
            })
            .unwrap_or_else(|| env::temp_dir().join("ollama-cowork/runs"));
        Ok(Self {
            opencode_executable,
            opencode_version: opencode_identity::VERSION.into(),
            broker_host_executable: assets.broker_host,
            node_executable,
            srt_bridge: assets.srt_bridge,
            docx_tool: assets.docx_tool,
            srt_win,
            srt_version: srt_identity::PACKAGE_VERSION.into(),
            ollama_origin,
            model,
            runs_root,
        })
    }

    pub fn runtime_settings(&self) -> RuntimeSettings {
        RuntimeSettings {
            opencode_executable: self.opencode_executable.clone(),
            opencode_version: self.opencode_version.clone(),
            opencode_sha256: opencode_identity::SHA256.into(),
            opencode_length: opencode_identity::FILE_LENGTH,
            broker_host_executable: self.broker_host_executable.clone(),
            node_executable: self.node_executable.clone(),
            srt_bridge: self.srt_bridge.clone(),
            docx_tool: self.docx_tool.clone(),
            srt_win: self.srt_win.clone(),
            srt_helper_sha256: srt_identity::SHA256.into(),
            srt_helper_length: srt_identity::FILE_LENGTH,
            ollama_origin: self.ollama_origin.clone(),
            model_id: self.model.clone(),
            startup_timeout: Duration::from_secs(20),
            tool_load_timeout: Duration::from_secs(60),
            request_timeout: Duration::from_secs(5),
            event_capacity: 256,
        }
    }
}
