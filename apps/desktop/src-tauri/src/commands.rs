use crate::{DesktopState, contracts::DesktopHealth, health};
use tauri::State;

#[tauri::command]
pub fn get_desktop_health(state: State<'_, DesktopState>) -> DesktopHealth {
    health::collect(&state.config)
}
