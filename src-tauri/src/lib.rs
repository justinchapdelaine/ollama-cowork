mod commands;
pub mod core;

use commands::{choose_workspace, probe_ollama, run_tool_probe};
use core::workspace::WorkspaceSelectionStore;

pub fn run() {
    tauri::Builder::default()
        .manage(WorkspaceSelectionStore::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            choose_workspace,
            probe_ollama,
            run_tool_probe
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Ollama Cowork");
}
