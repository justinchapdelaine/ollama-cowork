pub mod artifact_decoder;
mod commands;
pub mod composition;
mod config;
mod contracts;
mod health;

use config::DesktopConfig;
use contracts::DESKTOP_HEALTH_EVENT;
use tauri::{Emitter, Manager};

pub struct DesktopState {
    config: DesktopConfig,
}

pub fn run() {
    let config = DesktopConfig::load().expect("load validated desktop configuration");
    tauri::Builder::default()
        .manage(DesktopState { config })
        .invoke_handler(tauri::generate_handler![commands::get_desktop_health])
        .setup(|app| {
            let health = health::collect(&app.state::<DesktopState>().config);
            app.emit_to("main", DESKTOP_HEALTH_EVENT, health)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run Ollama Cowork desktop");
}
