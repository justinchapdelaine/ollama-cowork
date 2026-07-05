use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use crate::core::error::{AppError, AppResult};
use crate::core::messages::ToolResult;
use crate::core::model::ToolDefinition;
use crate::core::run::CancellationFlag;
use crate::core::workspace::WorkspaceContext;

const READ_FILE_MAX_BYTES: usize = 200_000;
const SEARCH_FILE_MAX_BYTES: usize = 200_000;
const SEARCH_MAX_FILES: usize = 400;
const SEARCH_MAX_MATCHES: usize = 80;

#[async_trait]
pub trait ToolRegistry: Send + Sync {
    async fn execute(
        &self,
        request: ToolExecutionRequest,
        cancellation: CancellationFlag,
    ) -> AppResult<ToolResult>;
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
        vec![
            list_files_tool_definition(),
            read_file_tool_definition(),
            search_files_tool_definition(),
        ]
    }

    fn execute_list_files(
        &self,
        request: ToolExecutionRequest,
        cancellation: &CancellationFlag,
    ) -> AppResult<ToolResult> {
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
            cancellation.check()?;
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

    fn execute_read_file(
        &self,
        request: ToolExecutionRequest,
        cancellation: &CancellationFlag,
    ) -> AppResult<ToolResult> {
        let path = request
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Runtime("read_file requires a string path".to_string()))?;

        let target = self.workspace.resolve_existing_relative_path(path)?;
        if !target.is_file() {
            return Err(AppError::Runtime(format!(
                "workspace path is not a file: {path}"
            )));
        }

        let read = read_bounded_file(&target, READ_FILE_MAX_BYTES, cancellation)?;
        let content = String::from_utf8_lossy(&read.bytes).to_string();

        Ok(ToolResult {
            call_id: request.call_id,
            name: "read_file".to_string(),
            content: json!({
                "path": path,
                "content": content,
                "truncated": read.truncated,
                "bytes_read": read.bytes.len(),
                "original_byte_count": read.original_byte_count,
            }),
        })
    }

    fn execute_search_files(
        &self,
        request: ToolExecutionRequest,
        cancellation: &CancellationFlag,
    ) -> AppResult<ToolResult> {
        let query = request
            .arguments
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Runtime("search_files requires a string query".to_string()))?;
        let path = request
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".");

        if query.trim().is_empty() {
            return Err(AppError::Runtime(
                "search_files query cannot be empty".to_string(),
            ));
        }

        let root = self.workspace.resolve_existing_relative_path(path)?;
        if !root.is_dir() {
            return Err(AppError::Runtime(format!(
                "workspace path is not a directory: {path}"
            )));
        }

        let mut search = SearchState::default();
        self.search_dir(&root, query, &mut search, cancellation)?;

        Ok(ToolResult {
            call_id: request.call_id,
            name: "search_files".to_string(),
            content: json!({
                "path": path,
                "query": query,
                "matches": search.matches,
                "searched_files": search.searched_files,
                "truncated": search.truncated,
            }),
        })
    }

    fn search_dir(
        &self,
        dir: &Path,
        query: &str,
        search: &mut SearchState,
        cancellation: &CancellationFlag,
    ) -> AppResult<()> {
        cancellation.check()?;
        if search.truncated {
            return Ok(());
        }

        let mut entries = fs::read_dir(dir)
            .map_err(|err| AppError::Runtime(err.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| AppError::Runtime(err.to_string()))?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            cancellation.check()?;
            if search.truncated {
                break;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry
                .file_type()
                .map_err(|err| AppError::Runtime(err.to_string()))?;

            if file_type.is_dir() {
                if should_hide_workspace_entry(&name) {
                    continue;
                }
                self.search_dir(&entry.path(), query, search, cancellation)?;
                continue;
            }

            if !file_type.is_file() || search.searched_files >= SEARCH_MAX_FILES {
                search.truncated = search.searched_files >= SEARCH_MAX_FILES;
                continue;
            }

            search.searched_files += 1;
            self.search_file(&entry.path(), query, search, cancellation)?;
        }

        Ok(())
    }

    fn search_file(
        &self,
        path: &Path,
        query: &str,
        search: &mut SearchState,
        cancellation: &CancellationFlag,
    ) -> AppResult<()> {
        let read = read_bounded_file(path, SEARCH_FILE_MAX_BYTES, cancellation)?;
        let content = String::from_utf8_lossy(&read.bytes);

        for (index, line) in content.lines().enumerate() {
            cancellation.check()?;
            if !line.contains(query) {
                continue;
            }

            let relative_path = path
                .strip_prefix(self.workspace.source_root())
                .map_err(|err| AppError::Runtime(err.to_string()))?
                .display()
                .to_string();
            search.matches.push(SearchMatch {
                path: relative_path,
                line_number: index + 1,
                line: line.to_string(),
            });

            if search.matches.len() >= SEARCH_MAX_MATCHES {
                search.truncated = true;
                break;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl ToolRegistry for LocalToolRegistry {
    async fn execute(
        &self,
        request: ToolExecutionRequest,
        cancellation: CancellationFlag,
    ) -> AppResult<ToolResult> {
        match request.name.as_str() {
            "list_files" => self.execute_list_files(request, &cancellation),
            "read_file" => self.execute_read_file(request, &cancellation),
            "search_files" => self.execute_search_files(request, &cancellation),
            name => Err(AppError::Runtime(format!("unknown tool: {name}"))),
        }
    }
}

fn read_bounded_file(
    path: &Path,
    max_bytes: usize,
    cancellation: &CancellationFlag,
) -> AppResult<BoundedRead> {
    cancellation.check()?;

    let mut file = File::open(path).map_err(|err| AppError::Runtime(err.to_string()))?;
    let original_byte_count = file
        .seek(SeekFrom::End(0))
        .map_err(|err| AppError::Runtime(err.to_string()))? as usize;
    file.seek(SeekFrom::Start(0))
        .map_err(|err| AppError::Runtime(err.to_string()))?;

    let mut bytes = Vec::with_capacity(max_bytes.min(original_byte_count));
    let mut take = file.take((max_bytes + 1) as u64);
    take.read_to_end(&mut bytes)
        .map_err(|err| AppError::Runtime(err.to_string()))?;
    cancellation.check()?;

    let truncated = bytes.len() > max_bytes;
    if truncated {
        bytes.truncate(max_bytes);
    }

    Ok(BoundedRead {
        bytes,
        original_byte_count,
        truncated,
    })
}

fn read_file_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "read_file".to_string(),
        description: "Read a UTF-8 text file from the selected workspace by relative path. Large files are truncated.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative file path to read."
                }
            },
            "required": ["path"]
        }),
    }
}

fn search_files_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "search_files".to_string(),
        description: "Search text files under a workspace-relative directory for an exact case-sensitive query. Generated and dependency directories are skipped.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative directory to search. Use . for the workspace root."
                },
                "query": {
                    "type": "string",
                    "description": "Exact text to search for."
                }
            },
            "required": ["query"]
        }),
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SearchState {
    searched_files: usize,
    matches: Vec<SearchMatch>,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchMatch {
    path: String,
    line_number: usize,
    line: String,
}

#[derive(Debug)]
struct BoundedRead {
    bytes: Vec<u8>,
    original_byte_count: usize,
    truncated: bool,
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
            .execute(request, CancellationFlag::default())
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
            .execute(request, CancellationFlag::default())
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
            .execute(request, CancellationFlag::default())
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
            .execute(request, CancellationFlag::default())
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

    #[tokio::test]
    async fn read_file_returns_text_content() {
        let registry = LocalToolRegistry::new(test_workspace());
        let request = ToolExecutionRequest {
            call_id: Some("call_test".to_string()),
            name: "read_file".to_string(),
            arguments: json!({ "path": "Cargo.toml" }),
        };

        let result = registry
            .execute(request, CancellationFlag::default())
            .await
            .expect("safe file read should succeed");
        assert_eq!(result.name, "read_file");
        assert!(result.content["content"]
            .as_str()
            .expect("content string")
            .contains("[package]"));
    }

    #[test]
    fn bounded_read_reports_truncation_without_returning_extra_bytes() {
        let path = test_workspace().source_root().join("Cargo.toml");
        let read = read_bounded_file(&path, 8, &CancellationFlag::default()).expect("bounded read");

        assert_eq!(read.bytes.len(), 8);
        assert!(read.original_byte_count >= read.bytes.len());
        assert!(read.truncated);
    }

    #[tokio::test]
    async fn search_files_returns_matching_lines() {
        let registry = LocalToolRegistry::new(test_workspace());
        let request = ToolExecutionRequest {
            call_id: Some("call_test".to_string()),
            name: "search_files".to_string(),
            arguments: json!({ "path": ".", "query": "Ollama" }),
        };

        let result = registry
            .execute(request, CancellationFlag::default())
            .await
            .expect("safe search should succeed");
        assert_eq!(result.name, "search_files");
        assert!(result.content["matches"].is_array());
    }

    fn test_workspace() -> WorkspaceContext {
        WorkspaceContext::new(PathBuf::from(".")).expect("test workspace")
    }
}
