use ollama_cowork_opencode_client::locate_executable;
use std::{env, path::PathBuf};

#[derive(Clone, Debug)]
pub struct DesktopConfig {
    pub opencode_executable: PathBuf,
    pub opencode_version: String,
    pub srt_win: PathBuf,
    pub srt_version: String,
    pub ollama_origin: String,
    pub model: String,
}

impl DesktopConfig {
    pub fn load() -> Result<Self, String> {
        let fallback = env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("npm/node_modules/opencode-ai/bin/opencode.exe"));
        let opencode_executable = locate_executable(
            env::var_os("OLLAMA_COWORK_OPENCODE").map(PathBuf::from),
            "opencode.exe",
            env::var_os("PATH"),
            &fallback.into_iter().collect::<Vec<_>>(),
        );
        let srt_win = env::var_os("OLLAMA_COWORK_SRT_WIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(r"C:\Program Files\ollama-cowork-spike\srt\0.0.65\srt-win.exe")
            });
        let ollama_origin = env::var("OLLAMA_COWORK_OLLAMA_ORIGIN")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".into());
        let model = env::var("OLLAMA_COWORK_OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:12b".into());
        Ok(Self {
            opencode_executable,
            opencode_version: "1.17.18".into(),
            srt_win,
            srt_version: "0.0.65".into(),
            ollama_origin,
            model,
        })
    }
}
