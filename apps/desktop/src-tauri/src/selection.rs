use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[cfg(windows)]
use std::{os::windows::fs::MetadataExt, path::Prefix};

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

pub trait DocumentPathPolicy: Send + Sync {
    fn canonicalize_docx(&self, path: &Path) -> Result<PathBuf, String>;
}

#[derive(Default)]
pub struct LocalDocxPathPolicy;

impl DocumentPathPolicy for LocalDocxPathPolicy {
    fn canonicalize_docx(&self, path: &Path) -> Result<PathBuf, String> {
        reject_unsupported_windows_path(path)?;
        let canonical = fs::canonicalize(path)
            .map_err(|error| format!("selected document is unavailable: {error}"))?;
        reject_unsupported_windows_path(&canonical)?;
        if !canonical.is_file()
            || canonical
                .extension()
                .and_then(|value| value.to_str())
                .is_none_or(|value| !value.eq_ignore_ascii_case("docx"))
        {
            return Err("selected document must be an existing DOCX".into());
        }
        Ok(canonical)
    }
}

#[cfg(windows)]
fn reject_unsupported_windows_path(path: &Path) -> Result<(), String> {
    if let Some(prefix) = path.components().find_map(|component| match component {
        std::path::Component::Prefix(prefix) => Some(prefix.kind()),
        _ => None,
    }) && matches!(
        prefix,
        Prefix::UNC(..) | Prefix::VerbatimUNC(..) | Prefix::DeviceNS(..) | Prefix::Verbatim(..)
    ) {
        return Err("selected document must use a local disk path".into());
    }
    for ancestor in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(ancestor)
            && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err("selected document path must not contain a reparse point".into());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn reject_unsupported_windows_path(_: &Path) -> Result<(), String> {
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedDocument {
    pub selection_id: String,
    pub display_name: String,
}

struct RegisteredDocument {
    id: String,
    path: PathBuf,
}

pub struct DocumentSelections {
    selected: Mutex<Option<RegisteredDocument>>,
    path_policy: Arc<dyn DocumentPathPolicy>,
}

impl Default for DocumentSelections {
    fn default() -> Self {
        Self::new(Arc::new(LocalDocxPathPolicy))
    }
}

impl DocumentSelections {
    pub fn new(path_policy: Arc<dyn DocumentPathPolicy>) -> Self {
        Self {
            selected: Mutex::new(None),
            path_policy,
        }
    }

    pub fn register(&self, path: &Path) -> Result<SelectedDocument, String> {
        let path = self.path_policy.canonicalize_docx(path)?;
        let display_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "selected document name is not valid UTF-8".to_owned())?
            .to_owned();
        let id = random_id()?;
        *self
            .selected
            .lock()
            .map_err(|_| "document selection state is unavailable")? = Some(RegisteredDocument {
            id: id.clone(),
            path,
        });
        Ok(SelectedDocument {
            selection_id: id,
            display_name,
        })
    }

    pub fn consume(&self, selection_id: &str) -> Result<PathBuf, String> {
        let mut selected = self
            .selected
            .lock()
            .map_err(|_| "document selection state is unavailable")?;
        let registered = selected
            .take()
            .ok_or_else(|| "document selection is missing or already consumed".to_owned())?;
        if registered.id != selection_id {
            *selected = Some(registered);
            return Err("document selection does not match".into());
        }
        Ok(registered.path)
    }
}

fn random_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_is_canonical_backend_owned_and_single_use() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("selected.docx");
        fs::write(&source, b"fixture").unwrap();
        let selections = DocumentSelections::default();
        let selected = selections.register(&source).unwrap();
        assert_eq!(selected.display_name, "selected.docx");
        assert_eq!(
            selections.consume(&selected.selection_id).unwrap(),
            fs::canonicalize(source).unwrap()
        );
        assert!(selections.consume(&selected.selection_id).is_err());
    }

    #[test]
    fn a_wrong_id_does_not_consume_the_selection() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("selected.docx");
        fs::write(&source, b"fixture").unwrap();
        let selections = DocumentSelections::default();
        let selected = selections.register(&source).unwrap();
        assert!(selections.consume("wrong").is_err());
        assert_eq!(
            selections.consume(&selected.selection_id).unwrap(),
            fs::canonicalize(source).unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    fn rejects_unc_and_device_paths_before_filesystem_access() {
        let policy = LocalDocxPathPolicy;
        assert!(
            policy
                .canonicalize_docx(Path::new(r"\\server\share\file.docx"))
                .is_err()
        );
        assert!(
            policy
                .canonicalize_docx(Path::new(r"\\.\C:\file.docx"))
                .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn rejects_reparse_point_selections() {
        use std::os::windows::fs::symlink_file;

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target.docx");
        let link = root.path().join("link.docx");
        fs::write(&target, b"fixture").unwrap();
        if symlink_file(&target, &link).is_err() {
            // Creating symlinks can require an elevated token when Windows
            // Developer Mode is disabled; UNC/device coverage still runs.
            return;
        }
        assert!(LocalDocxPathPolicy.canonicalize_docx(&link).is_err());
    }
}
