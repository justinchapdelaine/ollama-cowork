use ollama_cowork_core::{
    JobCancellation, JobCleanup, ModelEvent, ModelSession, WorkflowJobFactory,
};
use ollama_cowork_desktop_lib::composition::{
    DesktopWorkflowJobFactory, FilesystemJobWorkspaceFactory, FilesystemRuntimeAssetMaterializer,
    HttpRuntimeReadiness, LiveRuntimeProvisioner, RuntimeSettings,
    SupervisedRuntimeProcessLauncher, SystemJobSecretsGenerator, SystemLoopbackPortAllocator,
};
use ollama_cowork_desktop_lib::{opencode_identity, srt_identity};
use std::{
    env, fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn bounded_failure_logs(root: &std::path::Path) -> String {
    let mut pending = vec![root.to_path_buf()];
    let mut diagnostic = String::new();
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("log") {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            for line in contents.lines() {
                let lower = line.to_ascii_lowercase();
                if (lower.contains("error")
                    || lower.contains("fail")
                    || lower.contains("panic")
                    || lower.contains("exception"))
                    && !lower.contains("authorization")
                    && !lower.contains("password")
                    && !lower.contains("token")
                {
                    diagnostic.push_str(
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .as_ref(),
                    );
                    diagnostic.push_str(": ");
                    diagnostic.push_str(line);
                    diagnostic.push('\n');
                    if diagnostic.len() >= 8 * 1024 {
                        diagnostic.truncate(8 * 1024);
                        return diagnostic;
                    }
                }
            }
        }
    }
    diagnostic
}

fn required_path(name: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name} to run the live runtime test"))
}

fn live_settings(repo: &std::path::Path) -> RuntimeSettings {
    RuntimeSettings {
        opencode_executable: required_path("OLLAMA_COWORK_LIVE_OPENCODE"),
        opencode_version: "1.17.18".into(),
        opencode_sha256: opencode_identity::SHA256.into(),
        opencode_length: opencode_identity::FILE_LENGTH,
        broker_host_executable: repo.join("target/debug/ollama-cowork-broker-host.exe"),
        node_executable: required_path("OLLAMA_COWORK_LIVE_NODE"),
        srt_bridge: repo.join("scripts/runtime/srt-docx-bridge.mjs"),
        docx_tool: repo.join("target/debug/ollama-cowork-docx-tool.exe"),
        srt_win: required_path("OLLAMA_COWORK_LIVE_SRT_WIN"),
        srt_helper_sha256: srt_identity::SHA256.into(),
        srt_helper_length: srt_identity::FILE_LENGTH,
        ollama_origin: env::var("OLLAMA_COWORK_LIVE_OLLAMA_ORIGIN")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".into()),
        model_id: env::var("OLLAMA_COWORK_LIVE_MODEL").unwrap_or_else(|_| "gemma4:12b".into()),
        startup_timeout: Duration::from_secs(20),
        tool_load_timeout: Duration::from_secs(60),
        request_timeout: Duration::from_secs(10),
        event_capacity: 64,
    }
}

/// Opt-in host integration gate. It provisions real supervised broker and
/// opencode children but does not submit a model prompt or mutate a document.
#[test]
#[ignore = "requires pinned host executables and installed Windows SRT"]
fn provisions_and_cleans_up_the_live_runtime() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let temp = tempfile::tempdir().unwrap();
    let settings = live_settings(&repo);
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

/// Opt-in model-loop gate. It submits through the production opencode session
/// adapter in two fresh isolated sessions and waits for the first translated
/// event without authorizing a mutation.
#[test]
#[ignore = "requires the configured Ollama model and pinned host executables"]
fn submits_to_the_live_model_session() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for attempt in 1..=2 {
        let temp = tempfile::tempdir().unwrap();
        let runtime = LiveRuntimeProvisioner::new(
            live_settings(&repo),
            SystemLoopbackPortAllocator,
            FilesystemRuntimeAssetMaterializer::default(),
            SupervisedRuntimeProcessLauncher,
            HttpRuntimeReadiness,
        );
        let workspaces = FilesystemJobWorkspaceFactory::new(temp.path().join("runs")).unwrap();
        let job_id = format!("live-model-{attempt}");
        let job_workspace = workspaces.run_root().join(&job_id);
        let mut factory =
            DesktopWorkflowJobFactory::new(workspaces, SystemJobSecretsGenerator, runtime);
        let mut job = factory
            .create(
                &job_id,
                &repo.join("tests/fixtures/spike-001-original.docx"),
                &JobCancellation::default(),
            )
            .unwrap();

        job.session
            .submit("Rewrite the Summary section to be clearer and more concise.")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut assistant_text = String::new();
        let approval = loop {
            match job.session.next_event() {
                Err(error) => panic!(
                    "attempt {attempt}: {error}; bounded non-secret child errors:\n{}",
                    bounded_failure_logs(&job_workspace)
                ),
                Ok(Some(ModelEvent::ApprovalRequested { operation, .. })) => break operation,
                Ok(Some(ModelEvent::Text { text, .. })) => {
                    let remaining = 8 * 1024 - assistant_text.chars().count();
                    assistant_text.extend(text.chars().take(remaining));
                }
                Ok(Some(ModelEvent::Failed { code, message })) => {
                    panic!(
                        "attempt {attempt}: the live model failed before approval: {code}: {message}"
                    )
                }
                Ok(Some(ModelEvent::Idle)) => {
                    panic!(
                        "attempt {attempt}: the live model became idle without requesting DOCX approval; bounded assistant text: {assistant_text}"
                    )
                }
                Ok(Some(
                    ModelEvent::ToolStarted
                    | ModelEvent::ToolCompleted
                    | ModelEvent::ArtifactReady(_),
                )) => {
                    panic!(
                        "attempt {attempt}: the live model crossed the mutation boundary before approval"
                    )
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
                Ok(None) => panic!(
                    "attempt {attempt}: the live model produced no event before the deadline; bounded assistant text: {assistant_text}"
                ),
            }
        };
        assert!(matches!(
            approval,
            ollama_cowork_core::BrokerOperation::RewriteSection { .. }
        ));
        let _ = job.session.cancel();
        job.cleanup.terminate().unwrap();
    }
}
