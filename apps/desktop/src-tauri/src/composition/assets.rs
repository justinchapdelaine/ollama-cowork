use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const BASH_DENY_TOOL: &str = r#"import { tool } from "@opencode-ai/plugin"
export default tool({
  description: "Arbitrary shell execution is unavailable in Spike 001.",
  args: { command: tool.schema.string() },
  async execute() { throw new Error("arbitrary shell execution is unavailable in Spike 001") },
})
"#;

const DOCX_INSPECT_TOOL: &str = r#"import { tool } from "@opencode-ai/plugin"
const endpoint = () => {
  const url = process.env.OLLAMA_COWORK_BROKER_URL
  const token = process.env.OLLAMA_COWORK_BROKER_EXECUTION_TOKEN
  if (!url || !token || !/^http:\/\/127\.0\.0\.1:\d+$/.test(url)) throw new Error("trusted broker environment is invalid")
  return { url, token }
}
export default tool({
  description: "Inspect the selected DOCX through the trusted local broker.",
  args: {},
  async execute() {
    const { url, token } = endpoint()
    const response = await fetch(`${url}/execute`, { method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, body: JSON.stringify({ schema_version: 1, operation: "inspect" }) })
    if (!response.ok) throw new Error(`trusted broker rejected inspect: ${response.status}`)
    return JSON.stringify(await response.json())
  },
})
"#;

const DOCX_REWRITE_TOOL: &str = r#"import { tool } from "@opencode-ai/plugin"
export default tool({
  description: "Create a revised DOCX copy by replacing one unambiguous Heading1 section through the trusted broker.",
  args: { heading: tool.schema.string(), replacement_paragraphs: tool.schema.array(tool.schema.string()).min(1).max(32) },
  async execute(args, context) {
    await context.ask({ permission: "docx_rewrite_section", patterns: [args.heading], always: [], metadata: { operation: "rewrite_section", heading: args.heading, replacement_paragraphs: args.replacement_paragraphs } })
    const url = process.env.OLLAMA_COWORK_BROKER_URL
    const token = process.env.OLLAMA_COWORK_BROKER_EXECUTION_TOKEN
    if (!url || !token || !/^http:\/\/127\.0\.0\.1:\d+$/.test(url)) throw new Error("trusted broker environment is invalid")
    const response = await fetch(`${url}/execute`, { method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, body: JSON.stringify({ schema_version: 1, operation: "rewrite_section", heading: args.heading, replacement_paragraphs: args.replacement_paragraphs }) })
    if (!response.ok) throw new Error(`trusted broker rejected rewrite: ${response.status}`)
    return JSON.stringify(await response.json())
  },
})
"#;

#[derive(Clone, Debug)]
pub struct MaterializedRuntimeAssets {
    pub config_home: PathBuf,
    pub app_data: PathBuf,
    pub local_app_data: PathBuf,
    pub expected_tools: Vec<RuntimeToolIdentity>,
}

pub trait RuntimeAssetMaterializer: Send {
    fn materialize(&mut self, model_workspace: &Path) -> Result<MaterializedRuntimeAssets, String>;
}

pub struct RuntimeToolAsset {
    pub tool_id: &'static str,
    pub filename: &'static str,
    pub contents: &'static [u8],
    pub expected_description: &'static str,
    pub expected_parameters: &'static [RuntimeToolParameter],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeToolIdentity {
    pub tool_id: &'static str,
    pub description: &'static str,
    pub parameters: &'static [RuntimeToolParameter],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeToolParameter {
    pub name: &'static str,
    pub kind: RuntimeToolParameterKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeToolParameterKind {
    String,
    StringArray { min_items: u64, max_items: u64 },
}

pub trait RuntimeToolBundle: Send {
    fn tools(&self) -> &'static [RuntimeToolAsset];
}

#[derive(Default)]
pub struct Spike001DocxToolBundle;

impl RuntimeToolBundle for Spike001DocxToolBundle {
    fn tools(&self) -> &'static [RuntimeToolAsset] {
        static BASH_PARAMETERS: [RuntimeToolParameter; 1] = [RuntimeToolParameter {
            name: "command",
            kind: RuntimeToolParameterKind::String,
        }];
        static REWRITE_PARAMETERS: [RuntimeToolParameter; 2] = [
            RuntimeToolParameter {
                name: "heading",
                kind: RuntimeToolParameterKind::String,
            },
            RuntimeToolParameter {
                name: "replacement_paragraphs",
                kind: RuntimeToolParameterKind::StringArray {
                    min_items: 1,
                    max_items: 32,
                },
            },
        ];
        static TOOLS: [RuntimeToolAsset; 3] = [
            RuntimeToolAsset {
                tool_id: "bash",
                filename: "bash.ts",
                contents: BASH_DENY_TOOL.as_bytes(),
                expected_description: "Arbitrary shell execution is unavailable in Spike 001.",
                expected_parameters: &BASH_PARAMETERS,
            },
            RuntimeToolAsset {
                tool_id: "docx_inspect",
                filename: "docx_inspect.ts",
                contents: DOCX_INSPECT_TOOL.as_bytes(),
                expected_description: "Inspect the selected DOCX through the trusted local broker.",
                expected_parameters: &[],
            },
            RuntimeToolAsset {
                tool_id: "docx_rewrite_section",
                filename: "docx_rewrite_section.ts",
                contents: DOCX_REWRITE_TOOL.as_bytes(),
                expected_description: "Create a revised DOCX copy by replacing one unambiguous Heading1 section through the trusted broker.",
                expected_parameters: &REWRITE_PARAMETERS,
            },
        ];
        &TOOLS
    }
}

pub struct FilesystemRuntimeAssetMaterializer {
    bundle: Box<dyn RuntimeToolBundle>,
}

impl FilesystemRuntimeAssetMaterializer {
    pub fn new(bundle: impl RuntimeToolBundle + 'static) -> Self {
        Self {
            bundle: Box::new(bundle),
        }
    }
}

impl Default for FilesystemRuntimeAssetMaterializer {
    fn default() -> Self {
        Self::new(Spike001DocxToolBundle)
    }
}

impl RuntimeAssetMaterializer for FilesystemRuntimeAssetMaterializer {
    fn materialize(&mut self, model_workspace: &Path) -> Result<MaterializedRuntimeAssets, String> {
        let tools = model_workspace.join(".opencode").join("tools");
        let config_home = model_workspace.join("config-home");
        let app_data = model_workspace.join("appdata");
        let local_app_data = model_workspace.join("local-appdata");
        for directory in [&tools, &config_home, &app_data, &local_app_data] {
            fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        }
        for asset in self.bundle.tools() {
            write_new(&tools.join(asset.filename), asset.contents)?;
        }
        Ok(MaterializedRuntimeAssets {
            config_home,
            app_data,
            local_app_data,
            expected_tools: self
                .bundle
                .tools()
                .iter()
                .map(|asset| RuntimeToolIdentity {
                    tool_id: asset.tool_id,
                    description: asset.expected_description,
                    parameters: asset.expected_parameters,
                })
                .collect(),
        })
    }
}

fn write_new(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    file.write_all(contents).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materializes_only_the_spike_tool_surface_and_isolated_config_roots() {
        let root = tempfile::tempdir().unwrap();
        let mut materializer = FilesystemRuntimeAssetMaterializer::default();
        let assets = materializer.materialize(root.path()).unwrap();
        let tool_names: Vec<_> = fs::read_dir(root.path().join(".opencode/tools"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(tool_names.len(), 3);
        assert!(assets.config_home.is_dir());
        assert!(assets.app_data.is_dir());
        assert!(assets.local_app_data.is_dir());
        assert_eq!(assets.expected_tools.len(), 3);
        assert_eq!(assets.expected_tools[1].tool_id, "docx_inspect");
        assert!(materializer.materialize(root.path()).is_err());
    }
}
