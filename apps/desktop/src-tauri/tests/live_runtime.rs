use ollama_cowork_core::{JobCancellation, JobCleanup, WorkflowJobFactory};
use ollama_cowork_desktop_lib::composition::{
    DesktopWorkflowJobFactory, FilesystemJobWorkspaceFactory, FilesystemRuntimeAssetMaterializer,
    HttpRuntimeReadiness, LiveRuntimeProvisioner, RuntimeSettings,
    SupervisedRuntimeProcessLauncher, SystemJobSecretsGenerator, SystemLoopbackPortAllocator,
};
use std::{env, path::PathBuf, time::Duration};

fn required_path(name: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name} to run the live runtime test"))
}

/// Opt-in host integration gate. It provisions real supervised broker and
/// opencode children but does not submit a model prompt or mutate a document.
#[test]
#[ignore = "requires pinned host executables and installed Windows SRT"]
fn provisions_and_cleans_up_the_live_runtime() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let temp = tempfile::tempdir().unwrap();
    let settings = RuntimeSettings {
        opencode_executable: required_path("OLLAMA_COWORK_LIVE_OPENCODE"),
        opencode_version: "1.17.18".into(),
        broker_host_executable: repo.join("target/debug/ollama-cowork-broker-host.exe"),
        node_executable: required_path("OLLAMA_COWORK_LIVE_NODE"),
        srt_bridge: repo.join("scripts/runtime/srt-docx-bridge.mjs"),
        docx_tool: repo.join("target/debug/ollama-cowork-docx-tool.exe"),
        srt_win: required_path("OLLAMA_COWORK_LIVE_SRT_WIN"),
        ollama_origin: env::var("OLLAMA_COWORK_LIVE_OLLAMA_ORIGIN")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".into()),
        model_id: env::var("OLLAMA_COWORK_LIVE_MODEL").unwrap_or_else(|_| "gemma4:12b".into()),
        startup_timeout: Duration::from_secs(20),
        request_timeout: Duration::from_secs(10),
        event_capacity: 64,
    };
    let runs = temp.path().join("runs");
    let runtime = LiveRuntimeProvisioner::new(
        settings,
        SystemLoopbackPortAllocator,
        FilesystemRuntimeAssetMaterializer::default(),
        SupervisedRuntimeProcessLauncher,
        HttpRuntimeReadiness,
    );
    let workspaces = FilesystemJobWorkspaceFactory::new(runs).unwrap();
    let job_workspace = workspaces.run_root().join("live-runtime");
    let mut factory =
        DesktopWorkflowJobFactory::new(workspaces, SystemJobSecretsGenerator, runtime);
    let mut job = factory
        .create(
            "live-runtime",
            &repo.join("tests/fixtures/spike-001-original.docx"),
            &JobCancellation::default(),
        )
        .unwrap();
    job.cleanup.terminate().unwrap();
    assert!(!job_workspace.exists());
}
