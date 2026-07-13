mod api;
mod event_buffer;
mod model_session;
mod process;
mod sse;
mod translate;

pub use api::{OpencodeApi, OpencodeApiConfig, OpencodeApiError, StreamCancellation};
pub use event_buffer::{
    EventBufferError, ModelEventReceiver, ModelEventSender, bounded_model_event_channel,
};
pub use model_session::{
    EventCallback, EventReadyCallback, EventStreamFuture, OpencodeEventStream, OpencodeModelConfig,
    OpencodeModelSession, OpencodeSessionCommands, OpencodeSessionProvisioner,
};
pub use process::{
    OpencodeProcess, OpencodeProcessConfig, ProcessError, locate_executable, require_version,
};
pub use sse::{OpencodeEvent, SseDecoder, read_sse, read_sse_cancellable};
pub use translate::{OpencodeEventTranslator, ValidatedArtifactDecoder};
