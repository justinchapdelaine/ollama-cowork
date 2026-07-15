use crate::{
    EventBufferError, ModelEventReceiver, OpencodeApi, OpencodeEvent, OpencodeEventTranslator,
    OpencodePromptProfile, StreamCancellation, bounded_model_event_channel,
};
use ollama_cowork_core::{ModelEvent, ModelSession};
use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

const MAX_STREAM_DIAGNOSTIC_CHARS: usize = 1024;

fn stream_failure(error: &str) -> String {
    let diagnostic = error
        .chars()
        .filter(|value| *value != '\0')
        .take(MAX_STREAM_DIAGNOSTIC_CHARS)
        .collect::<String>();
    if diagnostic.trim().is_empty() {
        "opencode event stream failed".into()
    } else {
        format!("opencode event stream failed: {}", diagnostic.trim())
    }
}

pub type EventCallback<'a> = Box<dyn FnMut(OpencodeEvent) -> bool + Send + 'a>;
pub type EventReadyCallback<'a> = Box<dyn FnOnce() + Send + 'a>;
pub type EventStreamFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

pub trait OpencodeSessionCommands: Send + Sync + 'static {
    fn prompt_async(
        &self,
        session_id: &str,
        provider_id: &str,
        model_id: &str,
        profile: &OpencodePromptProfile,
        instruction: &str,
    ) -> Result<(), String>;
    fn reply_permission(&self, permission_id: &str, reply: &str) -> Result<(), String>;
    fn abort(&self, session_id: &str) -> Result<bool, String>;
}

pub trait OpencodeSessionProvisioner: Send + Sync + 'static {
    fn create_session(&self, title: &str) -> Result<String, String>;
}

pub trait OpencodeEventStream: Send + Sync + 'static {
    fn stream_events<'a>(
        &'a self,
        cancellation: &'a StreamCancellation,
        on_ready: EventReadyCallback<'a>,
        callback: EventCallback<'a>,
    ) -> EventStreamFuture<'a>;
}

impl OpencodeSessionProvisioner for OpencodeApi {
    fn create_session(&self, title: &str) -> Result<String, String> {
        let value = OpencodeApi::create_session(self, title).map_err(|error| error.to_string())?;
        value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| "opencode session response omitted its identifier".into())
    }
}

impl OpencodeSessionCommands for OpencodeApi {
    fn prompt_async(
        &self,
        session_id: &str,
        provider_id: &str,
        model_id: &str,
        profile: &OpencodePromptProfile,
        instruction: &str,
    ) -> Result<(), String> {
        OpencodeApi::prompt_async(
            self,
            session_id,
            provider_id,
            model_id,
            profile,
            instruction,
        )
        .map_err(|error| error.to_string())
    }

    fn reply_permission(&self, permission_id: &str, reply: &str) -> Result<(), String> {
        OpencodeApi::reply_permission(self, permission_id, reply).map_err(|error| error.to_string())
    }

    fn abort(&self, session_id: &str) -> Result<bool, String> {
        OpencodeApi::abort(self, session_id).map_err(|error| error.to_string())
    }
}

impl OpencodeEventStream for OpencodeApi {
    fn stream_events<'a>(
        &'a self,
        cancellation: &'a StreamCancellation,
        on_ready: EventReadyCallback<'a>,
        callback: EventCallback<'a>,
    ) -> EventStreamFuture<'a> {
        Box::pin(async move {
            OpencodeApi::stream_events(self, cancellation, on_ready, callback)
                .await
                .map_err(|error| {
                    let display = error.to_string();
                    let debug = format!("{error:?}");
                    format!(
                        "{display}; detail: {}",
                        debug
                            .chars()
                            .take(MAX_STREAM_DIAGNOSTIC_CHARS)
                            .collect::<String>()
                    )
                })
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpencodeModelConfig {
    pub session_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub prompt_profile: OpencodePromptProfile,
    pub event_capacity: usize,
    pub connect_timeout: std::time::Duration,
}

impl OpencodeModelConfig {
    fn validate(&self) -> Result<(), String> {
        if self.session_id.trim().is_empty()
            || self.provider_id.trim().is_empty()
            || self.model_id.trim().is_empty()
            || self.prompt_profile.agent_id().trim().is_empty()
        {
            return Err("opencode model configuration contains an empty identifier".into());
        }
        if self.event_capacity == 0 {
            return Err("opencode event capacity must be positive".into());
        }
        if self.connect_timeout.is_zero() {
            return Err("opencode event connection timeout must be positive".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PumpState {
    Running,
    Completed,
    Failed(String),
}

pub struct OpencodeModelSession<C, S>
where
    C: OpencodeSessionCommands,
    S: OpencodeEventStream,
{
    commands: Arc<C>,
    config: OpencodeModelConfig,
    cancellation: StreamCancellation,
    receiver: ModelEventReceiver,
    pump_state: Arc<Mutex<PumpState>>,
    pump: Option<JoinHandle<()>>,
    stream_type: PhantomData<S>,
}

impl<C, S> OpencodeModelSession<C, S>
where
    C: OpencodeSessionCommands,
    S: OpencodeEventStream,
{
    pub fn start(
        commands: Arc<C>,
        stream: Arc<S>,
        config: OpencodeModelConfig,
        mut translator: OpencodeEventTranslator,
    ) -> Result<Self, String> {
        config.validate()?;
        let (sender, receiver) = bounded_model_event_channel(config.event_capacity)
            .map_err(|error| error.to_string())?;
        let cancellation = StreamCancellation::default();
        let pump_cancellation = cancellation.clone();
        let pump_state = Arc::new(Mutex::new(PumpState::Running));
        let thread_state = pump_state.clone();
        let terminal_seen = Arc::new(AtomicBool::new(false));
        let thread_terminal_seen = terminal_seen.clone();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let connect_timeout = config.connect_timeout;
        let pump = thread::Builder::new()
            .name("ollama-cowork-opencode-events".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        *thread_state.lock().expect("pump state poisoned") =
                            PumpState::Failed("opencode event runtime failed".into());
                        return;
                    }
                };
                let on_ready: EventReadyCallback<'_> = Box::new(move || {
                    let _ = ready_sender.send(());
                });
                let callback_state = thread_state.clone();
                let callback_terminal_seen = thread_terminal_seen.clone();
                let callback: EventCallback<'_> = Box::new(move |raw_event| {
                    let events = translator.translate(raw_event);
                    let terminal = events
                        .iter()
                        .any(|event| matches!(event, ModelEvent::Idle | ModelEvent::Failed { .. }));
                    if terminal {
                        callback_terminal_seen.store(true, Ordering::Release);
                    }
                    for event in events {
                        if let Err(error) = sender.try_send(event) {
                            if error == EventBufferError::Full {
                                *callback_state.lock().expect("pump state poisoned") =
                                    PumpState::Failed("opencode event buffer exceeded".into());
                            }
                            return false;
                        }
                    }
                    !terminal
                });
                let result =
                    runtime.block_on(stream.stream_events(&pump_cancellation, on_ready, callback));
                let mut state = thread_state.lock().expect("pump state poisoned");
                if matches!(*state, PumpState::Running) {
                    *state = match result {
                        Ok(()) if thread_terminal_seen.load(Ordering::Acquire) => {
                            PumpState::Completed
                        }
                        Ok(()) if pump_cancellation.is_cancelled() => PumpState::Completed,
                        Ok(()) => PumpState::Failed(
                            "opencode event stream ended before a terminal event".into(),
                        ),
                        Err(error) => PumpState::Failed(stream_failure(&error)),
                    };
                }
            })
            .map_err(|error| error.to_string())?;
        if ready_receiver.recv_timeout(connect_timeout).is_err() {
            cancellation.cancel();
            return Err("opencode event stream did not become ready".into());
        }
        Ok(Self {
            commands,
            config,
            cancellation,
            receiver,
            pump_state,
            pump: Some(pump),
            stream_type: PhantomData,
        })
    }

    fn stop_pump(&mut self) -> Result<(), String> {
        self.cancellation.cancel();
        if let Some(pump) = self.pump.take() {
            pump.join()
                .map_err(|_| "opencode event pump panicked".to_owned())?;
        }
        Ok(())
    }
}

impl<C, S> ModelSession for OpencodeModelSession<C, S>
where
    C: OpencodeSessionCommands,
    S: OpencodeEventStream,
{
    fn submit(&mut self, instruction: &str) -> Result<(), String> {
        self.commands.prompt_async(
            &self.config.session_id,
            &self.config.provider_id,
            &self.config.model_id,
            &self.config.prompt_profile,
            instruction,
        )
    }

    fn decide(&mut self, external_id: &str, approved_once: bool) -> Result<(), String> {
        self.commands
            .reply_permission(external_id, if approved_once { "once" } else { "reject" })
    }

    fn cancel(&mut self) -> Result<(), String> {
        let abort = self.commands.abort(&self.config.session_id);
        let stop = self.stop_pump();
        match (abort, stop) {
            (Ok(true), Ok(())) => Ok(()),
            (Ok(false), _) => Err("opencode session was not running".into()),
            (Err(error), _) => Err(error),
            (Ok(true), Err(error)) => Err(error),
        }
    }

    fn next_event(&mut self) -> Result<Option<ModelEvent>, String> {
        match self.receiver.try_next() {
            Ok(Some(event)) => Ok(Some(event)),
            Ok(None) | Err(EventBufferError::Disconnected) => {
                match &*self.pump_state.lock().map_err(|_| "pump state poisoned")? {
                    PumpState::Failed(message) => Err(message.clone()),
                    PumpState::Running | PumpState::Completed => Ok(None),
                }
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

impl<C, S> Drop for OpencodeModelSession<C, S>
where
    C: OpencodeSessionCommands,
    S: OpencodeEventStream,
{
    fn drop(&mut self) {
        let _ = self.stop_pump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InspectedSections, ValidatedArtifactDecoder, ValidatedInspectionDecoder};
    use ollama_cowork_core::ArtifactMetadata;
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };

    #[derive(Default)]
    struct FakeApi {
        calls: Mutex<Vec<String>>,
        events: Mutex<VecDeque<OpencodeEvent>>,
        hold_open: AtomicBool,
        skip_ready: AtomicBool,
    }

    impl OpencodeSessionCommands for FakeApi {
        fn prompt_async(
            &self,
            session: &str,
            provider: &str,
            model: &str,
            profile: &OpencodePromptProfile,
            text: &str,
        ) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!(
                "prompt:{session}:{provider}:{model}:{}:{:?}:{text}",
                profile.agent_id(),
                profile.tool_overrides()
            ));
            Ok(())
        }
        fn reply_permission(&self, id: &str, reply: &str) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("reply:{id}:{reply}"));
            Ok(())
        }
        fn abort(&self, session: &str) -> Result<bool, String> {
            self.calls.lock().unwrap().push(format!("abort:{session}"));
            Ok(true)
        }
    }

    impl OpencodeEventStream for FakeApi {
        fn stream_events<'a>(
            &'a self,
            cancellation: &'a StreamCancellation,
            on_ready: EventReadyCallback<'a>,
            mut callback: EventCallback<'a>,
        ) -> EventStreamFuture<'a> {
            Box::pin(async move {
                if !self.skip_ready.load(Ordering::Acquire) {
                    on_ready();
                }
                loop {
                    if cancellation.is_cancelled() {
                        return Ok(());
                    }
                    let event = self.events.lock().unwrap().pop_front();
                    if let Some(event) = event {
                        if !callback(event) {
                            return Ok(());
                        }
                    } else if self.hold_open.load(Ordering::Acquire) {
                        tokio::task::yield_now().await;
                    } else {
                        return Ok(());
                    }
                }
            })
        }
    }

    struct Decoder;
    impl ValidatedArtifactDecoder for Decoder {
        fn decode_validated_artifact(
            &self,
            output: &str,
        ) -> Result<Option<ArtifactMetadata>, String> {
            Ok(
                (output == "artifact").then(|| ArtifactMetadata {
                    path: "input.revised.docx".into(),
                    media_type:
                        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                            .into(),
                    sha256: "abc".into(),
                }),
            )
        }
    }

    impl ValidatedInspectionDecoder for Decoder {
        fn decode_validated_inspection(&self, _: &str) -> Result<InspectedSections, String> {
            Ok(InspectedSections::from([(
                "Summary".into(),
                vec!["Current".into()],
            )]))
        }
    }

    fn raw(event_type: &str, properties: Value) -> OpencodeEvent {
        OpencodeEvent {
            event_type: event_type.into(),
            properties,
        }
    }

    fn config(capacity: usize) -> OpencodeModelConfig {
        OpencodeModelConfig {
            session_id: "session".into(),
            provider_id: "ollama-lan".into(),
            model_id: "gemma4:12b".into(),
            prompt_profile: OpencodePromptProfile::restricted(
                "spike-docx",
                "A DOCX is attached.",
                ["bash".into()],
            )
            .unwrap(),
            event_capacity: capacity,
            connect_timeout: Duration::from_secs(1),
        }
    }

    fn translator() -> OpencodeEventTranslator {
        OpencodeEventTranslator::new(
            "session".into(),
            "docx_rewrite_section".into(),
            "docx_rewrite_section".into(),
            "docx_inspect".into(),
            Box::new(Decoder),
            Box::new(Decoder),
        )
    }

    fn wait_event(session: &mut impl ModelSession) -> Result<ModelEvent, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(event) = session.next_event()? {
                return Ok(event);
            }
            if Instant::now() >= deadline {
                return Err("event timed out".into());
            }
            thread::yield_now();
        }
    }

    #[test]
    fn commands_are_forwarded_without_owning_broker_authorization() {
        let api = Arc::new(FakeApi::default());
        let mut session =
            OpencodeModelSession::start(api.clone(), api.clone(), config(8), translator()).unwrap();
        session.submit("rewrite").unwrap();
        session.decide("permission", true).unwrap();
        session.decide("permission-2", false).unwrap();
        assert_eq!(
            *api.calls.lock().unwrap(),
            vec![
                "prompt:session:ollama-lan:gemma4:12b:spike-docx:{\"bash\": false}:rewrite",
                "reply:permission:once",
                "reply:permission-2:reject",
            ]
        );
    }

    #[test]
    fn pump_translates_session_events_in_order() {
        let api = Arc::new(FakeApi::default());
        let inspection = json!({
            "schema_version": 1,
            "job_id": "job-1",
            "artifact": null,
            "result": json!({"schema_version":1,"status":"inspected","source_sha256":"abc","sections":[{"heading":"Summary","paragraphs":["Current"]}]}).to_string()
        }).to_string();
        api.events.lock().unwrap().extend([
            raw("message.part.updated", json!({"part":{"id":"inspect","sessionID":"session","type":"tool","tool":"docx_inspect","callID":"inspect-call","state":{"status":"completed","output":inspection}}})),
            raw("permission.asked", json!({"sessionID":"session","id":"permission","permission":"docx_rewrite_section","metadata":{"operation":"rewrite_section","heading":"Summary","replacement_paragraphs":["Revised"]}})),
            raw("message.part.updated", json!({"part":{"sessionID":"session","type":"tool","tool":"docx_rewrite_section","callID":"call","state":{"status":"running"}}})),
            raw("message.part.updated", json!({"part":{"sessionID":"session","type":"tool","tool":"docx_rewrite_section","callID":"call","state":{"status":"completed","output":"artifact"}}})),
            raw("session.idle", json!({"sessionID":"session"})),
        ]);
        let mut session =
            OpencodeModelSession::start(api.clone(), api, config(8), translator()).unwrap();
        assert!(matches!(
            wait_event(&mut session).unwrap(),
            ModelEvent::ApprovalRequested { .. }
        ));
        assert_eq!(wait_event(&mut session).unwrap(), ModelEvent::ToolCompleted);
        assert!(matches!(
            wait_event(&mut session).unwrap(),
            ModelEvent::ArtifactReady(_)
        ));
        assert_eq!(wait_event(&mut session).unwrap(), ModelEvent::Idle);
    }

    #[test]
    fn bounded_pump_reports_backpressure_after_buffered_events_are_drained() {
        let api = Arc::new(FakeApi::default());
        api.events.lock().unwrap().extend([
            raw(
                "message.updated",
                json!({"info":{"id":"assistant","sessionID":"session","role":"assistant"}}),
            ),
            raw(
                "message.part.updated",
                json!({"part":{"id":"one","messageID":"assistant","sessionID":"session","type":"text","text":"one"}}),
            ),
            raw(
                "message.part.updated",
                json!({"part":{"id":"two","messageID":"assistant","sessionID":"session","type":"text","text":"two"}}),
            ),
        ]);
        let mut session =
            OpencodeModelSession::start(api.clone(), api, config(1), translator()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(*session.pump_state.lock().unwrap(), PumpState::Running)
            && Instant::now() < deadline
        {
            thread::yield_now();
        }
        assert!(
            matches!(wait_event(&mut session).unwrap(), ModelEvent::Text { text, .. } if text == "one")
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match session.next_event() {
                Err(error) => {
                    assert_eq!(error, "opencode event buffer exceeded");
                    break;
                }
                Ok(None) if Instant::now() < deadline => thread::yield_now(),
                other => panic!("unexpected backpressure result: {other:?}"),
            }
        }
    }

    #[test]
    fn cancellation_aborts_session_and_joins_the_stream() {
        let api = Arc::new(FakeApi::default());
        api.hold_open.store(true, Ordering::Release);
        let mut session =
            OpencodeModelSession::start(api.clone(), api.clone(), config(8), translator()).unwrap();
        session.cancel().unwrap();
        assert_eq!(*api.calls.lock().unwrap(), vec!["abort:session"]);
    }

    #[test]
    fn premature_stream_end_becomes_an_adapter_failure() {
        let api = Arc::new(FakeApi::default());
        let mut session =
            OpencodeModelSession::start(api.clone(), api, config(8), translator()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match session.next_event() {
                Err(error) => {
                    assert_eq!(error, "opencode event stream ended before a terminal event");
                    break;
                }
                Ok(None) if Instant::now() < deadline => thread::yield_now(),
                other => panic!("unexpected stream completion result: {other:?}"),
            }
        }
    }

    #[test]
    fn construction_fails_if_the_event_stream_never_becomes_ready() {
        let api = Arc::new(FakeApi::default());
        api.skip_ready.store(true, Ordering::Release);
        api.hold_open.store(true, Ordering::Release);
        let mut bounded_config = config(8);
        bounded_config.connect_timeout = Duration::from_millis(20);
        let started = Instant::now();
        let result = OpencodeModelSession::start(api.clone(), api, bounded_config, translator());
        assert!(
            matches!(result, Err(error) if error == "opencode event stream did not become ready")
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn stream_failure_diagnostics_are_bounded() {
        assert_eq!(
            stream_failure("connection reset"),
            "opencode event stream failed: connection reset"
        );
        assert_eq!(stream_failure("\0"), "opencode event stream failed");
        assert!(
            stream_failure(&"x".repeat(MAX_STREAM_DIAGNOSTIC_CHARS + 20)).len()
                <= "opencode event stream failed: ".len() + MAX_STREAM_DIAGNOSTIC_CHARS
        );
    }
}
