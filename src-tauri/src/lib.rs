mod commands;
pub mod core;

use commands::{cancel_agent_run, choose_workspace, probe_ollama, run_agent_turn, run_tool_probe};
use core::{run::AgentRunStore, workspace::WorkspaceSelectionStore};

pub fn run() {
    tauri::Builder::default()
        .manage(WorkspaceSelectionStore::default())
        .manage(AgentRunStore::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            cancel_agent_run,
            choose_workspace,
            probe_ollama,
            run_agent_turn,
            run_tool_probe
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Ollama Cowork");
}
