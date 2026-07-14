use crate::{
    application::WorkflowApplication,
    composition::{
        DesktopWorkflowJobFactory, FilesystemJobWorkspaceFactory,
        FilesystemRuntimeAssetMaterializer, HttpRuntimeReadiness, LiveRuntimeProvisioner,
        SupervisedRuntimeProcessLauncher, SystemJobSecretsGenerator, SystemLoopbackPortAllocator,
    },
    config::DesktopConfig,
    selection::DocumentPathPolicy,
};
use ollama_cowork_core::{WorkflowController, WorkflowEventSink};
use std::sync::Arc;

pub fn build_workflow_application(
    config: &DesktopConfig,
    events: impl WorkflowEventSink + 'static,
    path_policy: Arc<dyn DocumentPathPolicy>,
) -> Result<WorkflowApplication, String> {
    let workspaces = FilesystemJobWorkspaceFactory::new(config.runs_root.clone())?;
    let runtime = LiveRuntimeProvisioner::new(
        config.runtime_settings(),
        SystemLoopbackPortAllocator,
        FilesystemRuntimeAssetMaterializer::default(),
        SupervisedRuntimeProcessLauncher,
        HttpRuntimeReadiness,
    );
    let factory = DesktopWorkflowJobFactory::new(workspaces, SystemJobSecretsGenerator, runtime);
    Ok(WorkflowApplication::with_path_policy(
        WorkflowController::new(factory, events),
        path_policy,
    ))
}
