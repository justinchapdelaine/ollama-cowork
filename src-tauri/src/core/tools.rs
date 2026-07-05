use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;

use crate::core::error::{AppError, AppResult};
use crate::core::messages::ToolResult;
use crate::core::model::ToolDefinition;
use crate::core::workspace::WorkspaceContext;

#[async_trait]
pub trait ToolRegistry: Send + Sync {
    async fn execute(&self, request: ToolExecutionRequest) -> AppResult<ToolResult>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionRequest {
    pub call_id: Option<String>,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone)]
pub struct LocalToolRegistry {
    workspace: WorkspaceContext,
}

impl LocalToolRegistry {
    pub fn new(workspace: WorkspaceContext) -> Self {
        Self { workspace }
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        vec![list_files_tool_definition()]
    }

    fn execute_list_files(&self, request: ToolExecutionRequest) -> AppResult<ToolResult> {
        let path = request
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Runtime("list_files requires a string path".to_string()))?;

        let target = self.workspace.resolve_existing_relative_path(path)?;
        if !target.is_dir() {
            return Err(AppError::Runtime(format!(
                "workspace path is not a directory: {path}"
            )));
        }

        let mut entries = Vec::new();
        let mut hidden_entries = Vec::new();
        for entry in fs::read_dir(target).map_err(|err| AppError::Runtime(err.to_string()))? {
            let entry = entry.map_err(|err| AppError::Runtime(err.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|err| AppError::Runtime(err.to_string()))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let is_hidden = should_hide_workspace_entry(&name);

            let entry = ListFilesEntry {
                name,
                kind: if file_type.is_dir() {
                    "directory".to_string()
                } else if file_type.is_file() {
                    "file".to_string()
                } else {
                    "other".to_string()
                },
            };

            if is_hidden {
                hidden_entries.push(entry);
                continue;
            }

            entries.push(entry);
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        hidden_entries.sort_by(|left, right| left.name.cmp(&right.name));

        Ok(ToolResult {
            call_id: request.call_id,
            name: "list_files".to_string(),
            content: json!({
                "path": path,
                "entries": entries,
                "hidden_entries": hidden_entries,
            }),
        })
    }
}

#[async_trait]
impl ToolRegistry for LocalToolRegistry {
    async fn execute(&self, request: ToolExecutionRequest) -> AppResult<ToolResult> {
        match request.name.as_str() {
            "list_files" => self.execute_list_files(request),
            name => Err(AppError::Runtime(format!("unknown tool: {name}"))),
        }
    }
}

fn list_files_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "list_files".to_string(),
        description: "List files and directories under a workspace-relative path. Broad listings omit known generated/noisy entries and report them separately as hidden_entries.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to list. Use . for the workspace root."
                }
            },
            "required": ["path"]
        }),
    }
}

fn should_hide_workspace_entry(name: &str) -> bool {
    matches!(name, ".git" | "dist" | "gen" | "node_modules" | "target")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ListFilesEntry {
    name: String,
    kind: String,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn list_files_rejects_workspace_escape_paths() {
        let registry = LocalToolRegistry::new(test_workspace());
        let request = ToolExecutionRequest {
            call_id: Some("call_test".to_string()),
            name: "list_files".to_string(),
            arguments: json!({ "path": ".." }),
        };

        let err = registry
            .execute(request)
            .await
            .expect_err("workspace escape paths must be rejected");
        assert!(err.to_string().contains("escapes selected workspace"));
    }

    #[tokio::test]
    async fn list_files_returns_root_entries() {
        let registry = LocalToolRegistry::new(test_workspace());
        let request = ToolExecutionRequest {
            call_id: Some("call_test".to_string()),
            name: "list_files".to_string(),
            arguments: json!({ "path": "." }),
        };

        let result = registry
            .execute(request)
            .await
            .expect("workspace root listing should succeed");
        assert_eq!(result.call_id.as_deref(), Some("call_test"));
        assert_eq!(result.name, "list_files");
        assert_eq!(result.content["path"], ".");
        assert!(result.content["entries"].is_array());
        assert!(result.content["hidden_entries"].is_array());
    }

    #[tokio::test]
    async fn list_files_accepts_safe_relative_subdirectories() {
        let registry = LocalToolRegistry::new(test_workspace());
        let request = ToolExecutionRequest {
            call_id: Some("call_test".to_string()),
            name: "list_files".to_string(),
            arguments: json!({ "path": "src" }),
        };

        let result = registry
            .execute(request)
            .await
            .expect("safe subdirectory listing should succeed");
        assert_eq!(result.content["path"], "src");
        assert!(result.content["entries"].is_array());
    }

    #[tokio::test]
    async fn list_files_reports_hidden_internal_entries_separately() {
        let registry = LocalToolRegistry::new(test_workspace());
        let request = ToolExecutionRequest {
            call_id: Some("call_test".to_string()),
            name: "list_files".to_string(),
            arguments: json!({ "path": "." }),
        };

        let result = registry
            .execute(request)
            .await
            .expect("workspace root listing should succeed");
        let visible_names = result.content["entries"]
            .as_array()
            .expect("entries array")
            .iter()
            .filter_map(|entry| entry["name"].as_str())
            .collect::<Vec<_>>();
        let hidden_names = result.content["hidden_entries"]
            .as_array()
            .expect("hidden entries array")
            .iter()
            .filter_map(|entry| entry["name"].as_str())
            .collect::<Vec<_>>();

        if PathBuf::from("target").exists() {
            assert!(!visible_names.contains(&"target"));
            assert!(hidden_names.contains(&"target"));
        }
    }

    fn test_workspace() -> WorkspaceContext {
        WorkspaceContext::new(PathBuf::from(".")).expect("test workspace")
    }
}
