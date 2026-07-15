use super::profile::{
    SPIKE_001_DOCX_AGENT_ID, SPIKE_001_DOCX_SYSTEM_PROMPT, spike_001_docx_prompt_profile,
};
use super::{
    CompositeJobCleanup, DynJobCleanup, DynModelSession, DynMutationAuthorization,
    LoopbackPortAllocator, MaterializedRuntimeAssets, ProvisionedRuntime, RuntimeAssetMaterializer,
    RuntimeProvisioner, RuntimeProvisioningRequest, RuntimeToolIdentity, RuntimeToolParameterKind,
};
use crate::artifact_decoder::TrustedDocxOutputDecoder;
use crate::opencode_identity;
use crate::srt_identity;
use ollama_cowork_broker_transport::{
    BrokerAuthorizationClient, BrokerAuthorizationConfig, broker_health_proof,
};
use ollama_cowork_core::{BROKER_SCHEMA_VERSION, JobCancellation, JobCleanup};
use ollama_cowork_opencode_client::{
    OpencodeApi, OpencodeApiConfig, OpencodeEventTranslator, OpencodeModelConfig,
    OpencodeModelSession, OpencodeProcess, OpencodeProcessConfig, OpencodeSessionProvisioner,
};
use ollama_cowork_process_supervisor::{
    ManagedChild, loopback_listener_owned_by, loopback_listener_owner,
};
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fmt, fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PROVIDER_ID: &str = "ollama-lan";
const PERMISSION_NAME: &str = "docx_rewrite_section";
const TOOL_NAME: &str = "docx_rewrite_section";
const READ_ONLY_TOOL_NAME: &str = "docx_inspect";
const MAX_ARTIFACT_BYTES: u64 = 50 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;
const MAX_CHILD_LOG_BYTES: u64 = 1024 * 1024;
const OPENCODE_START_ATTEMPTS: usize = 2;

#[derive(Clone, Debug)]
pub struct RuntimeSettings {
    pub opencode_executable: PathBuf,
    pub opencode_version: String,
    pub opencode_sha256: String,
    pub opencode_length: u64,
    pub broker_host_executable: PathBuf,
    pub node_executable: PathBuf,
    pub srt_bridge: PathBuf,
    pub docx_tool: PathBuf,
    pub srt_win: PathBuf,
    pub srt_helper_sha256: String,
    pub srt_helper_length: u64,
    pub ollama_origin: String,
    pub model_id: String,
    pub startup_timeout: Duration,
    pub tool_load_timeout: Duration,
    pub request_timeout: Duration,
    pub event_capacity: usize,
}

impl RuntimeSettings {
    pub fn required_files(&self) -> [(&'static str, &Path); 6] {
        [
            ("opencode", &self.opencode_executable),
            ("broker host", &self.broker_host_executable),
            ("Node.js", &self.node_executable),
            ("SRT bridge", &self.srt_bridge),
            ("DOCX tool", &self.docx_tool),
            ("SRT helper", &self.srt_win),
        ]
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, path) in self.required_files() {
            if !path.is_file() {
                return Err(format!("{name} is unavailable at {}", path.display()));
            }
        }
        if self.opencode_version.trim().is_empty()
            || self.opencode_sha256.trim().is_empty()
            || self.opencode_length == 0
            || self.model_id.trim().is_empty()
            || self.startup_timeout.is_zero()
            || self.tool_load_timeout.is_zero()
            || self.request_timeout.is_zero()
            || self.event_capacity == 0
        {
            return Err("runtime settings contain an empty or zero value".into());
        }
        validate_ollama_origin(&self.ollama_origin)?;
        self.validate_opencode_identity()?;
        if !srt_identity::matches_expected(
            &self.srt_win,
            self.srt_helper_length,
            &self.srt_helper_sha256,
        )? {
            return Err("SRT helper does not match the proof-tested package identity".into());
        }
        Ok(())
    }

    fn validate_opencode_identity(&self) -> Result<(), String> {
        if !opencode_identity::matches_expected(
            &self.opencode_executable,
            self.opencode_length,
            &self.opencode_sha256,
        )? {
            return Err("opencode does not match the proof-tested executable identity".into());
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct BrokerLaunchConfig {
    pub executable: PathBuf,
    pub bootstrap: BrokerBootstrap,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    pub log_limit_bytes: u64,
}

pub struct BrokerBootstrap(Vec<u8>);

impl BrokerBootstrap {
    fn serialize(config: &BrokerHostConfig<'_>) -> Result<Self, String> {
        serde_json::to_vec(config)
            .map(Self)
            .map_err(|error| error.to_string())
    }

    pub fn transfer_to(self, child: &mut ManagedChild) -> std::io::Result<()> {
        child.write_stdin_and_close(&self.0)
    }
}

impl fmt::Debug for BrokerBootstrap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrokerBootstrap([REDACTED])")
    }
}

pub trait RuntimeProcessLauncher: Send {
    fn start_broker(
        &mut self,
        config: BrokerLaunchConfig,
    ) -> Result<LaunchedRuntimeProcess, String>;
    fn start_opencode(
        &mut self,
        config: OpencodeProcessConfig,
    ) -> Result<LaunchedRuntimeProcess, String>;
}

pub struct LaunchedRuntimeProcess {
    pub process_id: u32,
    pub cleanup: DynJobCleanup,
}

#[derive(Default)]
pub struct SupervisedRuntimeProcessLauncher;

impl RuntimeProcessLauncher for SupervisedRuntimeProcessLauncher {
    fn start_broker(
        &mut self,
        config: BrokerLaunchConfig,
    ) -> Result<LaunchedRuntimeProcess, String> {
        let mut command = Command::new(config.executable);
        command.arg("--config-stdin").stdin(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        let mut child = ManagedChild::spawn_with_bounded_logs(
            &mut command,
            &config.stdout_log,
            &config.stderr_log,
            config.log_limit_bytes,
        )
        .map_err(|error| format!("could not start broker host: {error}"))?;
        if let Err(error) = config.bootstrap.transfer_to(&mut child) {
            let _ = child.stop();
            return Err(format!("could not transfer broker bootstrap: {error}"));
        }
        Ok(LaunchedRuntimeProcess {
            process_id: child.id(),
            cleanup: DynJobCleanup::new(child),
        })
    }

    fn start_opencode(
        &mut self,
        config: OpencodeProcessConfig,
    ) -> Result<LaunchedRuntimeProcess, String> {
        let process = OpencodeProcess::start(config).map_err(|error| error.to_string())?;
        Ok(LaunchedRuntimeProcess {
            process_id: process.id(),
            cleanup: DynJobCleanup::new(process),
        })
    }
}

pub trait RuntimeReadiness: Send {
    fn wait_for_broker(
        &mut self,
        base_url: &str,
        execution_token: &str,
        expected_process_id: u32,
        timeout: Duration,
        cancellation: &JobCancellation,
    ) -> Result<(), String>;
    fn wait_for_opencode(
        &mut self,
        api: &OpencodeApi,
        expected_process_id: u32,
        timeout: Duration,
        cancellation: &JobCancellation,
    ) -> Result<(), String>;
}

#[derive(Default)]
pub struct HttpRuntimeReadiness;

impl RuntimeReadiness for HttpRuntimeReadiness {
    fn wait_for_broker(
        &mut self,
        base_url: &str,
        execution_token: &str,
        expected_process_id: u32,
        timeout: Duration,
        cancellation: &JobCancellation,
    ) -> Result<(), String> {
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(1))
            .build()
            .map_err(|error| error.to_string())?;
        let mut challenge_bytes = [0_u8; 32];
        getrandom::fill(&mut challenge_bytes).map_err(|error| error.to_string())?;
        let challenge = challenge_bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let expected_proof = broker_health_proof(execution_token, &challenge);
        wait_until(timeout, cancellation, || {
            client
                .get(format!("{base_url}/health?challenge={challenge}"))
                .timeout(Duration::from_secs(2))
                .send()
                .ok()
                .filter(|response| response.status().is_success())
                .and_then(|response| {
                    serde_json::from_reader::<_, Value>(response.take(64 * 1024)).ok()
                })
                .is_some_and(|body| {
                    body.get("healthy").and_then(Value::as_bool) == Some(true)
                        && body.get("proof").and_then(Value::as_str)
                            == Some(expected_proof.as_str())
                })
                && loopback_listener_owned_by(
                    Url::parse(base_url).unwrap().port().unwrap(),
                    expected_process_id,
                )
                .is_ok_and(|owned| owned)
        })
        .then_some(())
        .ok_or_else(|| "broker host did not become ready".into())
    }

    fn wait_for_opencode(
        &mut self,
        api: &OpencodeApi,
        expected_process_id: u32,
        timeout: Duration,
        cancellation: &JobCancellation,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut last_error = "no HTTP response".to_owned();
        while Instant::now() < deadline {
            if cancellation.is_cancelled() {
                return Err("runtime provisioning cancelled".into());
            }
            match api.health() {
                Ok(body) if body.get("healthy").and_then(Value::as_bool) == Some(true) => {
                    let port = Url::parse(api.base_url()).unwrap().port().unwrap();
                    match loopback_listener_owned_by(port, expected_process_id) {
                        Ok(true) => return Ok(()),
                        Ok(false) => {
                            last_error = format!(
                                "listener owner {:?} is outside launched process tree rooted at {expected_process_id}",
                                loopback_listener_owner(port).ok().flatten()
                            )
                        }
                        Err(error) => last_error = format!("listener attestation failed: {error}"),
                    }
                }
                Ok(_) => last_error = "health response was not healthy".into(),
                Err(error) => last_error = error.to_string(),
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(format!("opencode did not become ready: {last_error}"))
    }
}

fn wait_until(
    timeout: Duration,
    cancellation: &JobCancellation,
    mut predicate: impl FnMut() -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cancellation.is_cancelled() {
            return false;
        }
        if predicate() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

pub struct LiveRuntimeProvisioner<P, A, L, H> {
    settings: RuntimeSettings,
    ports: P,
    assets: A,
    launcher: L,
    readiness: H,
}

struct ReadyOpencode {
    api: Arc<OpencodeApi>,
    cleanup: DynJobCleanup,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
}

impl<P, A, L, H> LiveRuntimeProvisioner<P, A, L, H> {
    pub fn new(settings: RuntimeSettings, ports: P, assets: A, launcher: L, readiness: H) -> Self {
        Self {
            settings,
            ports,
            assets,
            launcher,
            readiness,
        }
    }

    fn start_ready_opencode(
        &mut self,
        broker_port: u16,
        broker_url: &str,
        assets: &MaterializedRuntimeAssets,
        request: &RuntimeProvisioningRequest<'_>,
    ) -> Result<ReadyOpencode, String>
    where
        P: LoopbackPortAllocator,
        L: RuntimeProcessLauncher,
        H: RuntimeReadiness,
    {
        let model_secrets = request.secrets.model_process();
        let authorization = basic_authorization(model_secrets.opencode_password);
        let mut readiness_errors = Vec::new();

        for attempt in 0..OPENCODE_START_ATTEMPTS {
            ensure_not_cancelled(request.cancellation)?;
            // Re-attest at the execution boundary so a post-provisioning file
            // replacement cannot inherit trust from an earlier health check.
            self.settings.validate_opencode_identity()?;
            let opencode_port = allocate_distinct_port(&mut self.ports, broker_port)?;
            let opencode_url = format!("http://127.0.0.1:{opencode_port}");
            let api = Arc::new(
                OpencodeApi::new(OpencodeApiConfig {
                    base_url: opencode_url,
                    authorization: authorization.clone(),
                    timeout: self.settings.request_timeout,
                })
                .map_err(|error| error.to_string())?,
            );
            let suffix = if attempt == 0 {
                String::new()
            } else {
                format!("-attempt-{}", attempt + 1)
            };
            let stdout = request
                .workspace
                .root()
                .join(format!("opencode{suffix}-stdout.log"));
            let stderr = request
                .workspace
                .root()
                .join(format!("opencode{suffix}-stderr.log"));
            let mut process = self.launcher.start_opencode(OpencodeProcessConfig {
                executable: self.settings.opencode_executable.clone(),
                expected_version: self.settings.opencode_version.clone(),
                workspace: request.workspace.model().to_path_buf(),
                port: opencode_port,
                clear_environment: true,
                environment: opencode_environment(
                    &self.settings,
                    assets,
                    broker_url,
                    request.secrets,
                )?,
                stdout_log: Some(stdout.clone()),
                stderr_log: Some(stderr.clone()),
                log_limit_bytes: MAX_CHILD_LOG_BYTES,
            })?;
            match self.readiness.wait_for_opencode(
                &api,
                process.process_id,
                self.settings.startup_timeout,
                request.cancellation,
            ) {
                Ok(()) => {
                    return Ok(ReadyOpencode {
                        api,
                        cleanup: process.cleanup,
                        stdout_log: stdout,
                        stderr_log: stderr,
                    });
                }
                Err(error) => {
                    let diagnostic = startup_error(error, &[&stdout, &stderr], request.secrets);
                    let cleanup = process.cleanup.terminate().err();
                    readiness_errors.push(match cleanup {
                        Some(cleanup) => {
                            format!("{diagnostic}; opencode attempt cleanup failed: {cleanup}")
                        }
                        None => diagnostic,
                    });
                    if request.cancellation.is_cancelled() {
                        return Err("runtime provisioning cancelled".into());
                    }
                }
            }
        }

        Err(format!(
            "opencode startup attempts failed: {}",
            readiness_errors.join("; ")
        ))
    }
}

fn allocate_distinct_port(
    ports: &mut impl LoopbackPortAllocator,
    excluded: u16,
) -> Result<u16, String> {
    for _ in 0..4 {
        let port = ports.allocate()?;
        if port != excluded {
            return Ok(port);
        }
    }
    Err("could not allocate distinct runtime ports".into())
}

impl<P, A, L, H> RuntimeProvisioner for LiveRuntimeProvisioner<P, A, L, H>
where
    P: LoopbackPortAllocator,
    A: RuntimeAssetMaterializer,
    L: RuntimeProcessLauncher,
    H: RuntimeReadiness,
{
    fn provision(
        &mut self,
        request: RuntimeProvisioningRequest<'_>,
    ) -> Result<ProvisionedRuntime, String> {
        self.settings.validate()?;
        ensure_not_cancelled(request.cancellation)?;
        let assets = self.assets.materialize(request.workspace.model())?;
        let broker_port = self.ports.allocate()?;

        let broker_url = format!("http://127.0.0.1:{broker_port}");
        let source_sha256 = sha256_file(request.workspace.source())?;
        let broker_secrets = request.secrets.broker_bootstrap();
        let broker_stdout = request.workspace.root().join("broker-stdout.log");
        let broker_stderr = request.workspace.root().join("broker-stderr.log");
        let broker_bootstrap = BrokerBootstrap::serialize(&BrokerHostConfig {
            schema_version: BROKER_SCHEMA_VERSION,
            port: broker_port,
            auth_token: broker_secrets.execution_token,
            control_auth_token: broker_secrets.control_token,
            job_id: request.job_id,
            job_token: broker_secrets.job_token,
            source: request.workspace.source(),
            source_sha256: &source_sha256,
            private_output_directory: request.workspace.private_output(),
            publish_directory: request.workspace.publish(),
            node: &self.settings.node_executable,
            srt_bridge: &self.settings.srt_bridge,
            docx_tool: &self.settings.docx_tool,
            srt_win: &self.settings.srt_win,
            read_roots: &[request.workspace.root(), request.workspace.source()],
        })?;

        let mut resources: Vec<Box<dyn JobCleanup>> = Vec::new();
        let result = (|| {
            let broker_process = self.launcher.start_broker(BrokerLaunchConfig {
                executable: self.settings.broker_host_executable.clone(),
                bootstrap: broker_bootstrap,
                stdout_log: broker_stdout.clone(),
                stderr_log: broker_stderr.clone(),
                log_limit_bytes: MAX_CHILD_LOG_BYTES,
            })?;
            let broker_process_id = broker_process.process_id;
            resources.push(Box::new(broker_process.cleanup));
            self.readiness
                .wait_for_broker(
                    &broker_url,
                    broker_secrets.execution_token,
                    broker_process_id,
                    self.settings.startup_timeout,
                    request.cancellation,
                )
                .map_err(|error| {
                    startup_error(error, &[&broker_stdout, &broker_stderr], request.secrets)
                })?;

            let opencode =
                self.start_ready_opencode(broker_port, &broker_url, &assets, &request)?;
            let effective = opencode.api.config().map_err(|error| error.to_string())?;
            ensure_not_cancelled(request.cancellation)?;
            validate_effective_opencode_config(
                &effective,
                &self.settings.model_id,
                &self.settings.ollama_origin,
            )?;
            let tool_schemas = opencode
                .api
                .tool_schemas(
                    PROVIDER_ID,
                    &self.settings.model_id,
                    self.settings.tool_load_timeout,
                )
                .map_err(|error| {
                    let internal_log = assets
                        .config_home
                        .join(".local/share/opencode/log/opencode.log");
                    startup_error(
                        format!("opencode tool initialization failed: {error}"),
                        &[&opencode.stdout_log, &opencode.stderr_log, &internal_log],
                        request.secrets,
                    )
                })?;
            validate_tool_schemas(&tool_schemas, &assets.expected_tools)?;
            let api = opencode.api;
            resources.push(Box::new(opencode.cleanup));
            let session_id = OpencodeSessionProvisioner::create_session(
                api.as_ref(),
                "Ollama Cowork DOCX workflow",
            )?;
            let translator = OpencodeEventTranslator::new(
                session_id.clone(),
                PERMISSION_NAME.into(),
                TOOL_NAME.into(),
                READ_ONLY_TOOL_NAME.into(),
                Box::new(TrustedDocxOutputDecoder::new(
                    request.job_id.into(),
                    source_sha256.clone(),
                    request.workspace.publish().to_path_buf(),
                    MAX_ARTIFACT_BYTES,
                )),
                Box::new(TrustedDocxOutputDecoder::new(
                    request.job_id.into(),
                    source_sha256.clone(),
                    request.workspace.publish().to_path_buf(),
                    MAX_ARTIFACT_BYTES,
                )),
            );
            let session = OpencodeModelSession::start(
                api.clone(),
                api,
                OpencodeModelConfig {
                    session_id,
                    provider_id: PROVIDER_ID.into(),
                    model_id: self.settings.model_id.clone(),
                    prompt_profile: spike_001_docx_prompt_profile()?,
                    event_capacity: self.settings.event_capacity,
                    connect_timeout: self.settings.startup_timeout,
                },
                translator,
            )?;
            let control = BrokerAuthorizationClient::new(BrokerAuthorizationConfig {
                base_url: broker_url,
                control_authorization: format!("Bearer {}", request.secrets.broker_control().0),
                job_id: request.job_id.into(),
                timeout: self.settings.request_timeout,
            })
            .map_err(|error| error.to_string())?;
            Ok((session, control))
        })();

        match result {
            Ok((session, control)) => Ok(ProvisionedRuntime {
                session: DynModelSession::new(session),
                authorization: DynMutationAuthorization::new(control),
                cleanup: DynJobCleanup::new(CompositeJobCleanup::new(resources)),
            }),
            Err(error) => Err(rollback_resources(resources, error)),
        }
    }
}

fn ensure_not_cancelled(cancellation: &JobCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("runtime provisioning cancelled".into())
    } else {
        Ok(())
    }
}

fn rollback_resources(resources: Vec<Box<dyn JobCleanup>>, error: String) -> String {
    let mut cleanup = CompositeJobCleanup::new(resources);
    match cleanup.terminate() {
        Ok(()) => error,
        Err(cleanup_error) => format!("{error}; runtime rollback failed: {cleanup_error}"),
    }
}

fn startup_error(error: String, logs: &[&Path], secrets: &super::JobSecrets) -> String {
    let mut diagnostic = Vec::with_capacity(logs.len() * 4 * 1024);
    for path in logs {
        if let Ok(mut file) = fs::File::open(path) {
            if let Ok(metadata) = file.metadata() {
                let offset = metadata.len().saturating_sub(4 * 1024);
                let _ = file.seek(SeekFrom::Start(offset));
            }
            let _ = file.take(4 * 1024).read_to_end(&mut diagnostic);
        }
    }
    if diagnostic.is_empty() {
        return error;
    }
    diagnostic.retain(|byte| *byte != 0);
    let mut diagnostic = String::from_utf8_lossy(&diagnostic).into_owned();
    diagnostic = secrets.redact(&diagnostic);
    format!("{error}; bounded child diagnostic: {}", diagnostic.trim())
}

fn opencode_environment(
    settings: &RuntimeSettings,
    assets: &MaterializedRuntimeAssets,
    broker_url: &str,
    secrets: &super::JobSecrets,
) -> Result<HashMap<String, String>, String> {
    let model_secrets = secrets.model_process();
    let permission = json!({
        "*": "deny",
        "docx_inspect": "allow",
        PERMISSION_NAME: "ask"
    });
    let hidden_tools = json!({ "bash": false });
    let config = json!({
        "$schema": "https://opencode.ai/config.json",
        "model": format!("{PROVIDER_ID}/{}", settings.model_id),
        "autoupdate": false,
        "share": "disabled",
        "default_agent": SPIKE_001_DOCX_AGENT_ID,
        "permission": permission.clone(),
        "tools": hidden_tools.clone(),
        "agent": {
            SPIKE_001_DOCX_AGENT_ID: {
                "description": "Perform the single approved Spike 001 DOCX workflow.",
                "mode": "primary",
                "model": format!("{PROVIDER_ID}/{}", settings.model_id),
                "prompt": SPIKE_001_DOCX_SYSTEM_PROMPT,
                "permission": permission,
                "tools": hidden_tools
            }
        },
        "provider": {
            PROVIDER_ID: {
                "npm": "@ai-sdk/openai-compatible",
                "name": "Ollama (configured endpoint)",
                "options": { "baseURL": format!("{}/v1", settings.ollama_origin.trim_end_matches('/')) },
                "models": { &settings.model_id: { "name": &settings.model_id } }
            }
        }
    });
    let mut environment = HashMap::from([
        (
            "OPENCODE_SERVER_PASSWORD".into(),
            model_secrets.opencode_password.into(),
        ),
        (
            "OPENCODE_CONFIG_CONTENT".into(),
            serde_json::to_string(&config).map_err(|error| error.to_string())?,
        ),
        ("OPENCODE_DISABLE_MODELS_FETCH".into(), "true".into()),
        (
            "XDG_CONFIG_HOME".into(),
            assets.config_home.to_string_lossy().into_owned(),
        ),
        (
            "APPDATA".into(),
            assets.app_data.to_string_lossy().into_owned(),
        ),
        (
            "LOCALAPPDATA".into(),
            assets.local_app_data.to_string_lossy().into_owned(),
        ),
        (
            "HOME".into(),
            assets.config_home.to_string_lossy().into_owned(),
        ),
        (
            "USERPROFILE".into(),
            assets.config_home.to_string_lossy().into_owned(),
        ),
        ("OLLAMA_COWORK_BROKER_URL".into(), broker_url.into()),
        (
            "OLLAMA_COWORK_BROKER_EXECUTION_TOKEN".into(),
            model_secrets.broker_execution_token.into(),
        ),
    ]);
    #[cfg(windows)]
    {
        let isolated_home = assets.config_home.to_string_lossy();
        let bytes = isolated_home.as_bytes();
        if bytes.len() < 3 || bytes[1] != b':' || !matches!(bytes[2], b'\\' | b'/') {
            return Err("isolated opencode home must use an absolute Windows drive path".into());
        }
        environment.insert("HOMEDRIVE".into(), isolated_home[..2].into());
        environment.insert("HOMEPATH".into(), isolated_home[2..].into());
    }
    for name in [
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "Path",
        "PATHEXT",
        "COMSPEC",
        "OS",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_IDENTIFIER",
        "PROCESSOR_LEVEL",
        "PROCESSOR_REVISION",
        "USERNAME",
        "USERDOMAIN",
    ] {
        if let Ok(value) = std::env::var(name) {
            environment.insert(name.into(), value);
        }
    }
    Ok(environment)
}

#[derive(Serialize)]
struct BrokerHostConfig<'a> {
    schema_version: u32,
    port: u16,
    auth_token: &'a str,
    control_auth_token: &'a str,
    job_id: &'a str,
    job_token: &'a str,
    source: &'a Path,
    source_sha256: &'a str,
    private_output_directory: &'a Path,
    publish_directory: &'a Path,
    node: &'a Path,
    srt_bridge: &'a Path,
    docx_tool: &'a Path,
    srt_win: &'a Path,
    read_roots: &'a [&'a Path],
}

fn validate_ollama_origin(origin: &str) -> Result<(), String> {
    let url = Url::parse(origin).map_err(|error| format!("invalid Ollama origin: {error}"))?;
    if url.scheme() != "http"
        || url.host_str().is_none()
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Ollama origin must be an explicit http://host:port origin".into());
    }
    Ok(())
}

fn validate_effective_opencode_config(
    config: &Value,
    model_id: &str,
    ollama_origin: &str,
) -> Result<(), String> {
    let expected_model = format!("{PROVIDER_ID}/{model_id}");
    let permission = &config["permission"];
    let agent = &config["agent"][SPIKE_001_DOCX_AGENT_ID];
    let provider = &config["provider"][PROVIDER_ID];
    if config["model"] != expected_model
        || config["default_agent"] != SPIKE_001_DOCX_AGENT_ID
        || config["share"] != "disabled"
        || config["autoupdate"] != false
        || permission["*"] != "deny"
        || permission["docx_inspect"] != "allow"
        || permission[PERMISSION_NAME] != "ask"
        || config["tools"]["bash"] != false
        || agent["mode"] != "primary"
        || agent["model"] != expected_model
        || agent["prompt"] != SPIKE_001_DOCX_SYSTEM_PROMPT
        || agent["permission"] != *permission
        || agent["tools"] != config["tools"]
        || provider["npm"] != "@ai-sdk/openai-compatible"
        || provider["options"]["baseURL"] != format!("{}/v1", ollama_origin.trim_end_matches('/'))
        || provider["models"][model_id]["name"] != model_id
    {
        return Err(
            "effective opencode configuration is not the required Spike 001 profile".into(),
        );
    }
    Ok(())
}

fn validate_tool_schemas(
    inventory: &Value,
    expected_tools: &[RuntimeToolIdentity],
) -> Result<(), String> {
    let tools = inventory
        .as_array()
        .ok_or_else(|| "effective opencode tool inventory is not the expected array".to_owned())?;
    if expected_tools.is_empty() {
        return Err("required Spike 001 tool identity set is empty".into());
    }
    for expected in expected_tools {
        let matches = tools
            .iter()
            .filter(|tool| {
                tool.get("id").and_then(Value::as_str) == Some(expected.tool_id)
                    && tool.get("description").and_then(Value::as_str) == Some(expected.description)
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 || !tool_parameters_match(matches[0].get("parameters"), expected) {
            return Err(format!(
                "effective opencode tool inventory did not contain one exact '{}' schema",
                expected.tool_id
            ));
        }
    }
    Ok(())
}

fn tool_parameters_match(parameters: Option<&Value>, expected: &RuntimeToolIdentity) -> bool {
    let Some(parameters) = parameters else {
        return false;
    };
    let Some(properties) = parameters.get("properties").and_then(Value::as_object) else {
        return false;
    };
    let required = parameters.get("required").and_then(Value::as_array);
    if parameters.get("type").and_then(Value::as_str) != Some("object")
        || properties.len() != expected.parameters.len()
        || required.map(Vec::len).unwrap_or_default() != expected.parameters.len()
    {
        return false;
    }
    expected.parameters.iter().all(|parameter| {
        let Some(schema) = properties.get(parameter.name) else {
            return false;
        };
        if !required
            .into_iter()
            .flatten()
            .any(|name| name.as_str() == Some(parameter.name))
        {
            return false;
        }
        match parameter.kind {
            RuntimeToolParameterKind::String => {
                schema.get("type").and_then(Value::as_str) == Some("string")
            }
            RuntimeToolParameterKind::StringArray {
                min_items,
                max_items,
            } => {
                schema.get("type").and_then(Value::as_str) == Some("array")
                    && schema.get("minItems").and_then(Value::as_u64) == Some(min_items)
                    && schema.get("maxItems").and_then(Value::as_u64) == Some(max_items)
                    && schema
                        .get("items")
                        .and_then(|items| items.get("type"))
                        .and_then(Value::as_str)
                        == Some("string")
            }
        }
    })
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("could not open source DOCX for hashing: {error}"))?;
    let size = file
        .metadata()
        .map_err(|error| format!("could not inspect source DOCX: {error}"))?
        .len();
    if size > MAX_SOURCE_BYTES {
        return Err(format!("source DOCX exceeds {MAX_SOURCE_BYTES} bytes"));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("could not hash source DOCX: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn basic_authorization(password: &str) -> String {
    format!(
        "Basic {}",
        base64_encode(format!("opencode:{password}").as_bytes())
    )
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((value >> 18) & 63) as usize] as char);
        output.push(ALPHABET[((value >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    fn test_settings(root: &Path) -> RuntimeSettings {
        RuntimeSettings {
            opencode_executable: root.join("opencode.exe"),
            opencode_version: "1.17.18".into(),
            opencode_sha256: format!("{:X}", Sha256::digest(b"fixture")),
            opencode_length: 7,
            broker_host_executable: root.join("broker.exe"),
            node_executable: root.join("node.exe"),
            srt_bridge: root.join("bridge.mjs"),
            docx_tool: root.join("docx.exe"),
            srt_win: root.join("srt-win.exe"),
            srt_helper_sha256: format!("{:X}", Sha256::digest(b"fixture")),
            srt_helper_length: 7,
            ollama_origin: "http://192.0.2.125:11434".into(),
            model_id: "gemma4:12b".into(),
            startup_timeout: Duration::from_secs(1),
            tool_load_timeout: Duration::from_secs(1),
            request_timeout: Duration::from_secs(1),
            event_capacity: 8,
        }
    }

    fn materialize_setting_files(settings: &RuntimeSettings) {
        for path in [
            &settings.opencode_executable,
            &settings.broker_host_executable,
            &settings.node_executable,
            &settings.srt_bridge,
            &settings.docx_tool,
            &settings.srt_win,
        ] {
            fs::write(path, b"fixture").unwrap();
        }
    }

    struct Ports(VecDeque<u16>);
    impl LoopbackPortAllocator for Ports {
        fn allocate(&mut self) -> Result<u16, String> {
            self.0.pop_front().ok_or_else(|| "no test port".into())
        }
    }

    struct RecordedCleanup(Arc<Mutex<usize>>);
    impl JobCleanup for RecordedCleanup {
        fn terminate(&mut self) -> Result<(), String> {
            *self.0.lock().unwrap() += 1;
            Ok(())
        }
    }

    struct Launcher {
        cleanup_calls: Arc<Mutex<usize>>,
        fail_opencode: bool,
    }
    impl RuntimeProcessLauncher for Launcher {
        fn start_broker(
            &mut self,
            config: BrokerLaunchConfig,
        ) -> Result<LaunchedRuntimeProcess, String> {
            assert!(!config.bootstrap.0.is_empty());
            Ok(LaunchedRuntimeProcess {
                process_id: 1,
                cleanup: DynJobCleanup::new(RecordedCleanup(self.cleanup_calls.clone())),
            })
        }

        fn start_opencode(
            &mut self,
            config: OpencodeProcessConfig,
        ) -> Result<LaunchedRuntimeProcess, String> {
            assert!(config.clear_environment);
            if self.fail_opencode {
                Err("injected opencode launch failure".into())
            } else {
                Ok(LaunchedRuntimeProcess {
                    process_id: 2,
                    cleanup: DynJobCleanup::new(RecordedCleanup(self.cleanup_calls.clone())),
                })
            }
        }
    }

    struct Readiness(bool);
    impl RuntimeReadiness for Readiness {
        fn wait_for_broker(
            &mut self,
            _: &str,
            _: &str,
            _: u32,
            _: Duration,
            _: &JobCancellation,
        ) -> Result<(), String> {
            if self.0 {
                Err("injected broker readiness failure".into())
            } else {
                Ok(())
            }
        }

        fn wait_for_opencode(
            &mut self,
            _: &OpencodeApi,
            _: u32,
            _: Duration,
            _: &JobCancellation,
        ) -> Result<(), String> {
            unreachable!("opencode readiness is not reached by these rollback tests")
        }
    }

    struct RetryReadiness(VecDeque<Result<(), String>>);
    impl RuntimeReadiness for RetryReadiness {
        fn wait_for_broker(
            &mut self,
            _: &str,
            _: &str,
            _: u32,
            _: Duration,
            _: &JobCancellation,
        ) -> Result<(), String> {
            Ok(())
        }

        fn wait_for_opencode(
            &mut self,
            _: &OpencodeApi,
            _: u32,
            _: Duration,
            _: &JobCancellation,
        ) -> Result<(), String> {
            self.0.pop_front().expect("unexpected readiness call")
        }
    }

    fn rollback_fixture(
        broker_readiness_fails: bool,
        opencode_launch_fails: bool,
    ) -> (String, usize) {
        let root = tempfile::tempdir().unwrap();
        let settings = test_settings(root.path());
        materialize_setting_files(&settings);
        let source = root.path().join("source.docx");
        fs::write(&source, b"fixture").unwrap();
        let mut workspaces =
            super::super::FilesystemJobWorkspaceFactory::new(root.path().join("runs")).unwrap();
        let workspace =
            super::super::JobWorkspaceFactory::create(&mut workspaces, "job-1", &source).unwrap();
        let cleanup_calls = Arc::new(Mutex::new(0));
        let mut provisioner = LiveRuntimeProvisioner::new(
            settings,
            Ports(VecDeque::from([43123, 43124])),
            super::super::FilesystemRuntimeAssetMaterializer::default(),
            Launcher {
                cleanup_calls: cleanup_calls.clone(),
                fail_opencode: opencode_launch_fails,
            },
            Readiness(broker_readiness_fails),
        );
        let secrets = super::super::JobSecrets::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            "cccccccccccccccccccccccccccccccc".into(),
            "dddddddddddddddddddddddddddddddd".into(),
        )
        .unwrap();
        let error = provisioner
            .provision(RuntimeProvisioningRequest {
                job_id: "job-1",
                workspace: &workspace,
                secrets: &secrets,
                cancellation: &JobCancellation::default(),
            })
            .err()
            .expect("failure injection unexpectedly succeeded");
        let count = *cleanup_calls.lock().unwrap();
        (error, count)
    }

    #[test]
    fn basic_auth_matches_the_rfc_7617_wire_shape() {
        assert_eq!(basic_authorization("secret"), "Basic b3BlbmNvZGU6c2VjcmV0");
    }

    #[test]
    fn ollama_origin_requires_an_explicit_uncredentialed_http_origin() {
        assert!(validate_ollama_origin("http://192.0.2.125:11434").is_ok());
        for invalid in [
            "https://host:11434",
            "http://host",
            "http://user:secret@host:11434",
            "http://host:11434/v1",
            "http://host:11434?x=1",
        ] {
            assert!(validate_ollama_origin(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn readiness_waits_stop_when_startup_is_cancelled() {
        let cancellation = JobCancellation::default();
        cancellation.cancel();
        let mut calls = 0;
        assert!(!wait_until(Duration::from_secs(1), &cancellation, || {
            calls += 1;
            false
        }));
        assert_eq!(calls, 0);
    }

    #[test]
    fn opencode_readiness_gets_one_clean_fresh_port_retry() {
        let root = tempfile::tempdir().unwrap();
        let settings = test_settings(root.path());
        materialize_setting_files(&settings);
        let source = root.path().join("source.docx");
        fs::write(&source, b"fixture").unwrap();
        let mut workspaces =
            super::super::FilesystemJobWorkspaceFactory::new(root.path().join("runs")).unwrap();
        let workspace =
            super::super::JobWorkspaceFactory::create(&mut workspaces, "job-1", &source).unwrap();
        let assets = MaterializedRuntimeAssets {
            config_home: workspace.model().join("config"),
            app_data: workspace.model().join("appdata"),
            local_app_data: workspace.model().join("localappdata"),
            expected_tools: Vec::new(),
        };
        let cleanup_calls = Arc::new(Mutex::new(0));
        let mut provisioner = LiveRuntimeProvisioner::new(
            settings,
            Ports(VecDeque::from([43124, 43125])),
            super::super::FilesystemRuntimeAssetMaterializer::default(),
            Launcher {
                cleanup_calls: cleanup_calls.clone(),
                fail_opencode: false,
            },
            RetryReadiness(VecDeque::from([
                Err("first child did not become ready".into()),
                Ok(()),
            ])),
        );
        let secrets = super::super::JobSecrets::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            "cccccccccccccccccccccccccccccccc".into(),
            "dddddddddddddddddddddddddddddddd".into(),
        )
        .unwrap();
        let request = RuntimeProvisioningRequest {
            job_id: "job-1",
            workspace: &workspace,
            secrets: &secrets,
            cancellation: &JobCancellation::default(),
        };

        let mut ready = provisioner
            .start_ready_opencode(43123, "http://127.0.0.1:43123", &assets, &request)
            .unwrap();
        assert_eq!(*cleanup_calls.lock().unwrap(), 1);
        ready.cleanup.terminate().unwrap();
        assert_eq!(*cleanup_calls.lock().unwrap(), 2);
    }

    #[test]
    fn opencode_is_reattested_before_the_launcher_receives_it() {
        let root = tempfile::tempdir().unwrap();
        let settings = test_settings(root.path());
        materialize_setting_files(&settings);
        let source = root.path().join("source.docx");
        fs::write(&source, b"fixture").unwrap();
        let mut workspaces =
            super::super::FilesystemJobWorkspaceFactory::new(root.path().join("runs")).unwrap();
        let workspace =
            super::super::JobWorkspaceFactory::create(&mut workspaces, "job-1", &source).unwrap();
        let assets = MaterializedRuntimeAssets {
            config_home: workspace.model().join("config"),
            app_data: workspace.model().join("appdata"),
            local_app_data: workspace.model().join("localappdata"),
            expected_tools: Vec::new(),
        };
        fs::write(&settings.opencode_executable, b"changed").unwrap();
        let cleanup_calls = Arc::new(Mutex::new(0));
        let mut provisioner = LiveRuntimeProvisioner::new(
            settings,
            Ports(VecDeque::from([43124])),
            super::super::FilesystemRuntimeAssetMaterializer::default(),
            Launcher {
                cleanup_calls: cleanup_calls.clone(),
                fail_opencode: false,
            },
            RetryReadiness(VecDeque::new()),
        );
        let secrets = super::super::JobSecrets::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            "cccccccccccccccccccccccccccccccc".into(),
            "dddddddddddddddddddddddddddddddd".into(),
        )
        .unwrap();
        let error = provisioner
            .start_ready_opencode(
                43123,
                "http://127.0.0.1:43123",
                &assets,
                &RuntimeProvisioningRequest {
                    job_id: "job-1",
                    workspace: &workspace,
                    secrets: &secrets,
                    cancellation: &JobCancellation::default(),
                },
            )
            .err()
            .expect("changed executable unexpectedly reached the launcher");
        assert!(error.contains("proof-tested executable identity"));
        assert_eq!(*cleanup_calls.lock().unwrap(), 0);
    }

    #[test]
    fn opencode_environment_duplicates_default_deny_and_withholds_control_secrets() {
        let root = tempfile::tempdir().unwrap();
        let assets = MaterializedRuntimeAssets {
            config_home: root.path().join("config"),
            app_data: root.path().join("appdata"),
            local_app_data: root.path().join("localappdata"),
            expected_tools: Vec::new(),
        };
        let secrets = super::super::JobSecrets::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            "cccccccccccccccccccccccccccccccc".into(),
            "dddddddddddddddddddddddddddddddd".into(),
        )
        .unwrap();
        let environment = opencode_environment(
            &test_settings(root.path()),
            &assets,
            "http://127.0.0.1:43123",
            &secrets,
        )
        .unwrap();
        let config: Value = serde_json::from_str(&environment["OPENCODE_CONFIG_CONTENT"]).unwrap();
        assert_eq!(config["default_agent"], "spike-docx");
        assert_eq!(config["permission"]["*"], "deny");
        assert_eq!(config["tools"]["bash"], false);
        assert_eq!(
            config["agent"][SPIKE_001_DOCX_AGENT_ID]["prompt"],
            SPIKE_001_DOCX_SYSTEM_PROMPT
        );
        assert_eq!(
            config["agent"]["spike-docx"]["permission"],
            config["permission"]
        );
        assert_eq!(config["share"], "disabled");
        assert_eq!(environment["OPENCODE_DISABLE_MODELS_FETCH"], "true");
        #[cfg(windows)]
        {
            let home = assets.config_home.to_string_lossy();
            assert_eq!(
                format!("{}{}", environment["HOMEDRIVE"], environment["HOMEPATH"]),
                home
            );
        }
        assert_eq!(
            environment["OLLAMA_COWORK_BROKER_EXECUTION_TOKEN"],
            secrets.model_process().broker_execution_token
        );
        let serialized = serde_json::to_string(&environment).unwrap();
        assert!(!serialized.contains(secrets.broker_control().0));
        assert!(!serialized.contains(secrets.broker_bootstrap().job_token));
        validate_effective_opencode_config(&config, "gemma4:12b", "http://192.0.2.125:11434")
            .unwrap();
    }

    #[test]
    fn effective_config_validation_fails_on_broadened_agent_policy() {
        let mut config = json!({
            "model": "ollama-lan/gemma4:12b",
            "default_agent": "spike-docx",
            "share": "disabled",
            "autoupdate": false,
            "permission": {"*":"deny","docx_inspect":"allow","docx_rewrite_section":"ask"},
            "tools": {"bash":false},
            "agent": {"spike-docx": {
                "mode":"primary",
                "model":"ollama-lan/gemma4:12b",
                "prompt": SPIKE_001_DOCX_SYSTEM_PROMPT,
                "permission":{"*":"deny","docx_inspect":"allow","docx_rewrite_section":"ask"},
                "tools":{"bash":false}
            }},
            "provider": {"ollama-lan": {
                "npm":"@ai-sdk/openai-compatible",
                "options":{"baseURL":"http://192.0.2.125:11434/v1"},
                "models":{"gemma4:12b":{"name":"gemma4:12b"}}
            }}
        });
        validate_effective_opencode_config(&config, "gemma4:12b", "http://192.0.2.125:11434")
            .unwrap();
        config["agent"]["spike-docx"]["permission"]["bash"] = json!("allow");
        assert!(
            validate_effective_opencode_config(&config, "gemma4:12b", "http://192.0.2.125:11434")
                .is_err()
        );
    }

    #[test]
    fn tool_schema_validation_requires_exact_id_description_and_parameters() {
        static BASH_PARAMETERS: [super::super::RuntimeToolParameter; 1] =
            [super::super::RuntimeToolParameter {
                name: "command",
                kind: RuntimeToolParameterKind::String,
            }];
        static REWRITE_PARAMETERS: [super::super::RuntimeToolParameter; 2] = [
            super::super::RuntimeToolParameter {
                name: "heading",
                kind: RuntimeToolParameterKind::String,
            },
            super::super::RuntimeToolParameter {
                name: "replacement_paragraphs",
                kind: RuntimeToolParameterKind::StringArray {
                    min_items: 1,
                    max_items: 32,
                },
            },
        ];
        let schemas = json!([
            {"id":"bash","description":"deny marker","parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}},
            {"id":"docx_inspect","description":"inspect marker","parameters":{"type":"object","properties":{},"required":[]}},
            {"id":"docx_rewrite_section","description":"rewrite marker","parameters":{"type":"object","properties":{"heading":{"type":"string"},"replacement_paragraphs":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":32}},"required":["heading","replacement_paragraphs"]}}
        ]);
        let expected = [
            RuntimeToolIdentity {
                tool_id: "bash",
                description: "deny marker",
                parameters: &BASH_PARAMETERS,
            },
            RuntimeToolIdentity {
                tool_id: "docx_inspect",
                description: "inspect marker",
                parameters: &[],
            },
            RuntimeToolIdentity {
                tool_id: "docx_rewrite_section",
                description: "rewrite marker",
                parameters: &REWRITE_PARAMETERS,
            },
        ];
        validate_tool_schemas(&schemas, &expected).unwrap();

        let mut built_in_same_id = schemas.clone();
        built_in_same_id.as_array_mut().unwrap().push(json!({
            "id":"bash",
            "description":"built-in shell tool",
            "parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}
        }));
        validate_tool_schemas(&built_in_same_id, &expected).unwrap();

        let mut swapped = schemas.clone();
        swapped[0]["description"] = json!("inspect marker");
        assert!(validate_tool_schemas(&swapped, &expected).is_err());

        let mut duplicate = schemas.clone();
        let duplicate_entry = duplicate[0].clone();
        duplicate.as_array_mut().unwrap().push(duplicate_entry);
        assert!(validate_tool_schemas(&duplicate, &expected).is_err());

        let mut missing_parameters = schemas.clone();
        missing_parameters[1]
            .as_object_mut()
            .unwrap()
            .remove("parameters");
        assert!(validate_tool_schemas(&missing_parameters, &expected).is_err());

        let mut widened_parameters = schemas;
        widened_parameters[2]["parameters"]["properties"]["path"] = json!({"type":"string"});
        widened_parameters[2]["parameters"]["required"] =
            json!(["heading", "replacement_paragraphs", "path"]);
        assert!(validate_tool_schemas(&widened_parameters, &expected).is_err());
    }

    #[test]
    fn broker_readiness_failure_terminates_the_started_broker() {
        let (error, cleanup_calls) = rollback_fixture(true, false);
        assert!(error.contains("broker readiness failure"));
        assert_eq!(cleanup_calls, 1);
    }

    #[test]
    fn opencode_launch_failure_rolls_back_the_started_broker() {
        let (error, cleanup_calls) = rollback_fixture(false, true);
        assert!(error.contains("opencode launch failure"));
        assert_eq!(cleanup_calls, 1);
    }
}
