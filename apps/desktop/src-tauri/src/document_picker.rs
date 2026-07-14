use std::path::PathBuf;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

pub trait DocumentPicker: Send + Sync {
    fn pick_docx(&self) -> Result<Option<PathBuf>, String>;
}

pub struct TauriDocumentPicker {
    app: AppHandle,
}

impl TauriDocumentPicker {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl DocumentPicker for TauriDocumentPicker {
    fn pick_docx(&self) -> Result<Option<PathBuf>, String> {
        self.app
            .dialog()
            .file()
            .add_filter("Word document", &["docx"])
            .set_title("Select a DOCX to revise")
            .blocking_pick_file()
            .map(|path| path.into_path().map_err(|error| error.to_string()))
            .transpose()
    }
}
