mod commands;
pub mod core;

use commands::{probe_ollama, run_tool_probe};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![probe_ollama, run_tool_probe])
        .run(tauri::generate_context!())
        .expect("failed to run Ollama Cowork");
}
