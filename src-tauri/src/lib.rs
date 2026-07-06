mod commands;
pub mod core;

use commands::{
    append_session_event, cancel_agent_run, choose_workspace, create_session, list_sessions,
    load_session, probe_ollama, run_agent_turn, run_agent_turn_stream, run_tool_probe,
};
use core::{run::AgentRunStore, session::JsonlSessionStore, workspace::WorkspaceSelectionStore};
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .manage(WorkspaceSelectionStore::default())
        .manage(AgentRunStore::default())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let session_root = app.path().app_data_dir()?.join("sessions");
            app.manage(JsonlSessionStore::new(session_root));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            append_session_event,
            cancel_agent_run,
            choose_workspace,
            create_session,
            list_sessions,
            load_session,
            probe_ollama,
            run_agent_turn,
            run_agent_turn_stream,
            run_tool_probe
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Ollama Cowork");
}
