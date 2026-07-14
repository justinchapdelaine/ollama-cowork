use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct RuntimeAssetOverrides {
    pub broker_host: Option<PathBuf>,
    pub docx_tool: Option<PathBuf>,
    pub srt_bridge: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeAssetPaths {
    pub broker_host: PathBuf,
    pub docx_tool: PathBuf,
    pub srt_bridge: PathBuf,
}

pub trait RuntimeAssetResolver: Send + Sync {
    fn resolve(
        &self,
        executable_directory: &Path,
        overrides: RuntimeAssetOverrides,
    ) -> RuntimeAssetPaths;
}

/// Default layout prepared by the desktop build: trusted helpers sit beside
/// the desktop executable and may be replaced only by explicit host overrides.
#[derive(Default)]
pub struct SiblingRuntimeAssetResolver;

impl RuntimeAssetResolver for SiblingRuntimeAssetResolver {
    fn resolve(
        &self,
        executable_directory: &Path,
        overrides: RuntimeAssetOverrides,
    ) -> RuntimeAssetPaths {
        RuntimeAssetPaths {
            broker_host: overrides
                .broker_host
                .unwrap_or_else(|| executable_directory.join("ollama-cowork-broker-host.exe")),
            docx_tool: overrides
                .docx_tool
                .unwrap_or_else(|| executable_directory.join("ollama-cowork-docx-tool.exe")),
            srt_bridge: overrides
                .srt_bridge
                .unwrap_or_else(|| executable_directory.join("srt-docx-bridge.mjs")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_layout_is_relocatable_and_individually_overridable() {
        let paths = SiblingRuntimeAssetResolver.resolve(
            Path::new("C:/app"),
            RuntimeAssetOverrides {
                docx_tool: Some("D:/tools/docx.exe".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            paths.broker_host,
            PathBuf::from("C:/app/ollama-cowork-broker-host.exe")
        );
        assert_eq!(paths.docx_tool, PathBuf::from("D:/tools/docx.exe"));
        assert_eq!(
            paths.srt_bridge,
            PathBuf::from("C:/app/srt-docx-bridge.mjs")
        );
    }
}
