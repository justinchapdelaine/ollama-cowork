use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
