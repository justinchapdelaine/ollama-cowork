mod application;
pub mod artifact_decoder;
mod bootstrap;
mod commands;
pub mod composition;
mod config;
mod contracts;
mod document_picker;
mod events;
mod health;
pub mod opencode_identity;
mod runtime_assets;
mod selection;
pub mod srt_identity;

use config::DesktopConfig;
use contracts::DESKTOP_HEALTH_EVENT;
use health::{DesktopPrerequisiteProbe, SystemDesktopPrerequisiteProbe};
use selection::LocalDocxPathPolicy;
use std::sync::Arc;
use tauri::{Emitter, Manager, RunEvent};

pub struct DesktopState {
    config: DesktopConfig,
    health_probe: Arc<dyn DesktopPrerequisiteProbe>,
}

pub fn run() {
    let config = DesktopConfig::load().expect("load validated desktop configuration");
    let health_probe: Arc<dyn DesktopPrerequisiteProbe> = Arc::new(SystemDesktopPrerequisiteProbe);
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(DesktopState {
            config,
            health_probe,
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_desktop_health,
            commands::select_docx_document,
            commands::start_docx_workflow,
            commands::approve_docx_action_once,
            commands::reject_docx_action,
            commands::cancel_docx_workflow,
            commands::poll_docx_workflow,
        ])
        .setup(|app| {
            let state = app.state::<DesktopState>();
            let health = health::collect_with(&state.config, state.health_probe.as_ref());
            app.emit_to("main", DESKTOP_HEALTH_EVENT, health)?;
            let events = events::WorkflowEventBridge::new(events::TauriFrontendEventEmitter::new(
                app.handle().clone(),
            ));
            let workflow = bootstrap::build_workflow_application(
                &app.state::<DesktopState>().config,
                events,
                Arc::new(LocalDocxPathPolicy),
            )
            .map_err(std::io::Error::other)?;
            app.manage(Arc::new(workflow));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build Ollama Cowork desktop");
    app.run(|handle, event| {
        if matches!(event, RunEvent::ExitRequested { .. }) {
            let _ = handle
                .state::<Arc<application::WorkflowApplication>>()
                .shutdown();
        }
    });
}
