use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceContext {
    source_root: PathBuf,
}

impl WorkspaceContext {
    pub fn new(source_root: impl AsRef<Path>) -> AppResult<Self> {
        let root = source_root.as_ref();
        if root.as_os_str().is_empty() {
            return Err(AppError::InvalidConfig(
                "workspace root cannot be empty".to_string(),
            ));
        }

        let canonical = root.canonicalize().map_err(|err| {
            AppError::InvalidConfig(format!(
                "workspace root {} could not be resolved: {err}",
                root.display()
            ))
        })?;

        if !canonical.is_dir() {
            return Err(AppError::InvalidConfig(format!(
                "workspace root {} is not a directory",
                canonical.display()
            )));
        }

        Ok(Self {
            source_root: canonical,
        })
    }

    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    pub fn resolve_existing_relative_path(
        &self,
        relative_path: impl AsRef<Path>,
    ) -> AppResult<PathBuf> {
        let relative_path = relative_path.as_ref();
        if relative_path.as_os_str().is_empty() {
            return Err(AppError::InvalidConfig(
                "workspace-relative path cannot be empty".to_string(),
            ));
        }

        if relative_path.is_absolute() {
            return Err(AppError::PolicyDenied(format!(
                "workspace path must be relative, got {}",
                relative_path.display()
            )));
        }

        let resolved = self
            .source_root
            .join(relative_path)
            .canonicalize()
            .map_err(|err| {
                AppError::Runtime(format!(
                    "workspace path {} could not be resolved: {err}",
                    relative_path.display()
                ))
            })?;

        if !resolved.starts_with(&self.source_root) {
            return Err(AppError::PolicyDenied(format!(
                "workspace path escapes selected workspace: {}",
                relative_path.display()
            )));
        }

        Ok(resolved)
    }
}

#[derive(Debug, Default)]
pub struct WorkspaceSelectionStore {
    selections: Mutex<HashMap<Uuid, WorkspaceContext>>,
}

impl WorkspaceSelectionStore {
    pub fn insert(&self, workspace: WorkspaceContext) -> AppResult<WorkspaceSelection> {
        let id = Uuid::new_v4();
        let source_root = workspace.source_root().display().to_string();
        self.selections
            .lock()
            .map_err(|err| AppError::Runtime(format!("workspace selection store poisoned: {err}")))?
            .insert(id, workspace);

        Ok(WorkspaceSelection { id, source_root })
    }

    pub fn get(&self, id: Uuid) -> AppResult<WorkspaceContext> {
        self.selections
            .lock()
            .map_err(|err| AppError::Runtime(format!("workspace selection store poisoned: {err}")))?
            .get(&id)
            .cloned()
            .ok_or_else(|| AppError::PolicyDenied("unknown workspace selection".to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSelection {
    pub id: Uuid,
    pub source_root: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_selection_store_returns_selected_workspace_by_id() {
        let store = WorkspaceSelectionStore::default();
        let workspace = WorkspaceContext::new(PathBuf::from(".")).expect("test workspace");
        let selection = store.insert(workspace.clone()).expect("insert selection");

        let stored = store.get(selection.id).expect("stored selection");
        assert_eq!(stored, workspace);
    }

    #[test]
    fn workspace_selection_store_rejects_unknown_ids() {
        let store = WorkspaceSelectionStore::default();
        let err = store
            .get(Uuid::new_v4())
            .expect_err("unknown ids must be rejected");

        assert!(err.to_string().contains("unknown workspace selection"));
    }
}
