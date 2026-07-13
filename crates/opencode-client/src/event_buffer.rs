use ollama_cowork_core::ModelEvent;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum EventBufferError {
    #[error("opencode event buffer capacity must be positive")]
    InvalidCapacity,
    #[error("opencode event buffer is full")]
    Full,
    #[error("opencode event buffer is disconnected")]
    Disconnected,
}

#[derive(Clone)]
pub struct ModelEventSender(SyncSender<ModelEvent>);
pub struct ModelEventReceiver(Receiver<ModelEvent>);

pub fn bounded_model_event_channel(
    capacity: usize,
) -> Result<(ModelEventSender, ModelEventReceiver), EventBufferError> {
    if capacity == 0 {
        return Err(EventBufferError::InvalidCapacity);
    }
    let (sender, receiver) = sync_channel(capacity);
    Ok((ModelEventSender(sender), ModelEventReceiver(receiver)))
}

impl ModelEventSender {
    pub fn try_send(&self, event: ModelEvent) -> Result<(), EventBufferError> {
        self.0.try_send(event).map_err(|error| match error {
            TrySendError::Full(_) => EventBufferError::Full,
            TrySendError::Disconnected(_) => EventBufferError::Disconnected,
        })
    }
}

impl ModelEventReceiver {
    pub fn try_next(&self) -> Result<Option<ModelEvent>, EventBufferError> {
        match self.0.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(EventBufferError::Disconnected),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_is_bounded_and_nonblocking() {
        let (sender, receiver) = bounded_model_event_channel(1).unwrap();
        sender.try_send(ModelEvent::Text("one".into())).unwrap();
        assert_eq!(
            sender.try_send(ModelEvent::Text("two".into())),
            Err(EventBufferError::Full)
        );
        assert!(matches!(receiver.try_next(), Ok(Some(ModelEvent::Text(text))) if text == "one"));
        assert_eq!(receiver.try_next(), Ok(None));
    }
}
