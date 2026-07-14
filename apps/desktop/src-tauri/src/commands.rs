use crate::{
    DesktopState,
    application::{ApplicationError, WorkflowApplication},
    contracts::DesktopHealth,
    document_picker::{DocumentPicker, TauriDocumentPicker},
    health,
    selection::SelectedDocument,
};
use ollama_cowork_core::WorkflowReceipt;
use serde::Deserialize;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
pub fn get_desktop_health(state: State<'_, DesktopState>) -> DesktopHealth {
    health::collect_with(&state.config, state.health_probe.as_ref())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartDocxRequest {
    selection_id: String,
    instruction: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDecisionRequest {
    job_id: String,
    action_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRequest {
    job_id: String,
}

#[tauri::command]
pub async fn select_docx_document(
    app: AppHandle,
    workflow: State<'_, Arc<WorkflowApplication>>,
) -> Result<Option<SelectedDocument>, ApplicationError> {
    let workflow = Arc::clone(&workflow);
    tauri::async_runtime::spawn_blocking(move || {
        let picker = TauriDocumentPicker::new(app);
        let Some(path) = picker
            .pick_docx()
            .map_err(|_| ApplicationError::state_unavailable())?
        else {
            return Ok(None);
        };
        workflow.register_selected_document(&path).map(Some)
    })
    .await
    .map_err(|_| ApplicationError::state_unavailable())?
}

#[tauri::command]
pub async fn start_docx_workflow(
    request: StartDocxRequest,
    workflow: State<'_, Arc<WorkflowApplication>>,
) -> Result<WorkflowReceipt, ApplicationError> {
    workflow.start_docx(request.selection_id, request.instruction)
}

#[tauri::command]
pub async fn approve_docx_action_once(
    request: ActionDecisionRequest,
    workflow: State<'_, Arc<WorkflowApplication>>,
) -> Result<WorkflowReceipt, ApplicationError> {
    let workflow = Arc::clone(&workflow);
    tauri::async_runtime::spawn_blocking(move || {
        workflow.approve_once(request.job_id, request.action_id)
    })
    .await
    .map_err(|_| ApplicationError::state_unavailable())?
}

#[tauri::command]
pub async fn reject_docx_action(
    request: ActionDecisionRequest,
    workflow: State<'_, Arc<WorkflowApplication>>,
) -> Result<WorkflowReceipt, ApplicationError> {
    let workflow = Arc::clone(&workflow);
    tauri::async_runtime::spawn_blocking(move || workflow.reject(request.job_id, request.action_id))
        .await
        .map_err(|_| ApplicationError::state_unavailable())?
}

#[tauri::command]
pub async fn cancel_docx_workflow(
    request: JobRequest,
    workflow: State<'_, Arc<WorkflowApplication>>,
) -> Result<WorkflowReceipt, ApplicationError> {
    let workflow = Arc::clone(&workflow);
    tauri::async_runtime::spawn_blocking(move || workflow.cancel(request.job_id))
        .await
        .map_err(|_| ApplicationError::state_unavailable())?
}

#[tauri::command]
pub async fn poll_docx_workflow(
    request: JobRequest,
    workflow: State<'_, Arc<WorkflowApplication>>,
) -> Result<(), ApplicationError> {
    let workflow = Arc::clone(&workflow);
    tauri::async_runtime::spawn_blocking(move || workflow.poll(&request.job_id))
        .await
        .map_err(|_| ApplicationError::state_unavailable())?
}
