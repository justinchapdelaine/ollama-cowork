use std::{
    cmp::Reverse,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::{AppError, AppResult};
use crate::core::messages::ConversationMessage;

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn create_session(&self, request: CreateSessionRequest) -> AppResult<SessionSnapshot>;
    async fn append_event(&self, session_id: SessionId, event: SessionEvent) -> AppResult<()>;
    async fn load_session(&self, session_id: SessionId) -> AppResult<SessionSnapshot>;
    async fn list_sessions(&self) -> AppResult<Vec<SessionSummary>>;
}

#[derive(Debug, Clone)]
pub struct JsonlSessionStore {
    root: PathBuf,
}

impl JsonlSessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn session_path(&self, session_id: SessionId) -> PathBuf {
        self.root.join(format!("{}.jsonl", session_id.0))
    }

    fn ensure_root(&self) -> AppResult<()> {
        fs::create_dir_all(&self.root)
            .map_err(|err| AppError::SessionStore(format!("create session directory: {err}")))
    }

    fn create_log(&self, session_id: SessionId, record: &SessionRecord) -> AppResult<()> {
        self.ensure_root()?;
        let path = self.session_path(session_id);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|err| AppError::SessionStore(format!("create session log: {err}")))?;
        serde_json::to_writer(&mut file, record)
            .map_err(|err| AppError::SessionStore(format!("serialize session record: {err}")))?;
        file.write_all(b"\n")
            .map_err(|err| AppError::SessionStore(format!("write session record: {err}")))
    }

    fn append_record(&self, session_id: SessionId, record: &SessionRecord) -> AppResult<()> {
        let path = self.session_path(session_id);
        self.read_snapshot(&path)?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|err| AppError::SessionStore(format!("open session log: {err}")))?;
        serde_json::to_writer(&mut file, record)
            .map_err(|err| AppError::SessionStore(format!("serialize session record: {err}")))?;
        file.write_all(b"\n")
            .map_err(|err| AppError::SessionStore(format!("write session record: {err}")))
    }

    fn read_snapshot(&self, path: &Path) -> AppResult<SessionSnapshot> {
        let file = File::open(path)
            .map_err(|err| AppError::SessionStore(format!("open session log: {err}")))?;
        let reader = BufReader::new(file);
        let mut snapshot: Option<SessionSnapshot> = None;

        for line in reader.lines() {
            let line =
                line.map_err(|err| AppError::SessionStore(format!("read session log: {err}")))?;
            if line.trim().is_empty() {
                continue;
            }

            let record = serde_json::from_str::<SessionRecord>(&line)
                .map_err(|err| AppError::SessionStore(format!("parse session log: {err}")))?;
            match record {
                SessionRecord::Created(created) => {
                    snapshot = Some(SessionSnapshot {
                        id: created.id,
                        title: created.title,
                        workspace_root: created.workspace_root,
                        base_url: created.base_url,
                        model: created.model,
                        created_at_ms: created.created_at_ms,
                        updated_at_ms: created.created_at_ms,
                        messages: Vec::new(),
                        events: Vec::new(),
                    });
                }
                SessionRecord::Event { event, at_ms } => {
                    let snapshot = snapshot.as_mut().ok_or_else(|| {
                        AppError::SessionStore(
                            "session log event appeared before created record".to_string(),
                        )
                    })?;
                    apply_event(snapshot, event, at_ms);
                }
            }
        }

        snapshot.ok_or_else(|| AppError::SessionStore("session log is empty".to_string()))
    }
}

#[async_trait]
impl SessionStore for JsonlSessionStore {
    async fn create_session(&self, request: CreateSessionRequest) -> AppResult<SessionSnapshot> {
        let id = SessionId(Uuid::new_v4());
        let created_at_ms = now_ms();
        let created = SessionCreatedRecord {
            id,
            title: request.title,
            workspace_root: request.workspace_root,
            base_url: request.base_url,
            model: request.model,
            created_at_ms,
        };
        self.create_log(id, &SessionRecord::Created(created.clone()))?;

        Ok(SessionSnapshot {
            id,
            title: created.title,
            workspace_root: created.workspace_root,
            base_url: created.base_url,
            model: created.model,
            created_at_ms,
            updated_at_ms: created_at_ms,
            messages: Vec::new(),
            events: Vec::new(),
        })
    }

    async fn append_event(&self, session_id: SessionId, event: SessionEvent) -> AppResult<()> {
        self.append_record(
            session_id,
            &SessionRecord::Event {
                event,
                at_ms: now_ms(),
            },
        )
    }

    async fn load_session(&self, session_id: SessionId) -> AppResult<SessionSnapshot> {
        self.read_snapshot(&self.session_path(session_id))
    }

    async fn list_sessions(&self) -> AppResult<Vec<SessionSummary>> {
        self.ensure_root()?;
        let mut summaries = Vec::new();

        for entry in fs::read_dir(&self.root)
            .map_err(|err| AppError::SessionStore(format!("read session directory: {err}")))?
        {
            let entry = entry
                .map_err(|err| AppError::SessionStore(format!("read session entry: {err}")))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }

            let Ok(snapshot) = self.read_snapshot(&path) else {
                continue;
            };
            summaries.push(SessionSummary {
                id: snapshot.id,
                title: snapshot.title,
                workspace_root: snapshot.workspace_root,
                model: snapshot.model,
                created_at_ms: snapshot.created_at_ms,
                updated_at_ms: snapshot.updated_at_ms,
                message_count: snapshot.messages.len(),
            });
        }

        summaries.sort_by_key(|summary| Reverse(summary.updated_at_ms));
        Ok(summaries)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    pub title: String,
    pub workspace_root: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub id: SessionId,
    pub title: String,
    pub workspace_root: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub messages: Vec<ConversationMessage>,
    pub events: Vec<TimestampedSessionEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: SessionId,
    pub title: String,
    pub workspace_root: Option<String>,
    pub model: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimestampedSessionEvent {
    pub at_ms: u64,
    pub event: SessionEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    AgentTurnCompleted {
        run_id: Uuid,
        transport: AgentTurnTransport,
        base_url: Option<String>,
        model: Option<String>,
        messages: Vec<ConversationMessage>,
        done_reason: Option<String>,
        tool_iteration_count: usize,
    },
    AgentTurnFailed {
        run_id: Uuid,
        message: String,
    },
    AgentTurnCancelled {
        run_id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnTransport {
    Streaming,
    NonStreamingFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
enum SessionRecord {
    Created(SessionCreatedRecord),
    Event { event: SessionEvent, at_ms: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionCreatedRecord {
    id: SessionId,
    title: String,
    workspace_root: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    created_at_ms: u64,
}

fn apply_event(snapshot: &mut SessionSnapshot, event: SessionEvent, at_ms: u64) {
    if let SessionEvent::AgentTurnCompleted {
        base_url,
        model,
        messages,
        ..
    } = &event
    {
        if let Some(base_url) = base_url {
            snapshot.base_url = Some(base_url.clone());
        }
        if let Some(model) = model {
            snapshot.model = Some(model.clone());
        }
        snapshot.messages.extend(messages.iter().cloned());
    }

    snapshot.updated_at_ms = at_ms;
    snapshot
        .events
        .push(TimestampedSessionEvent { at_ms, event });
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_millis()
        .try_into()
        .expect("epoch milliseconds should fit in u64")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{MessagePart, MessageRole};

    #[tokio::test]
    async fn jsonl_session_store_replays_completed_turns() {
        let store = JsonlSessionStore::new(test_session_dir());
        let session = store
            .create_session(CreateSessionRequest {
                title: "Test session".to_string(),
                workspace_root: Some("workspace".to_string()),
                base_url: Some("http://127.0.0.1:11434".to_string()),
                model: Some("test-model".to_string()),
            })
            .await
            .expect("create session");
        let message = text_message(MessageRole::User, "hello");

        store
            .append_event(
                session.id,
                SessionEvent::AgentTurnCompleted {
                    run_id: Uuid::new_v4(),
                    transport: AgentTurnTransport::Streaming,
                    base_url: Some("http://127.0.0.1:11434".to_string()),
                    model: Some("updated-model".to_string()),
                    messages: vec![message.clone()],
                    done_reason: Some("stop".to_string()),
                    tool_iteration_count: 0,
                },
            )
            .await
            .expect("append event");

        let loaded = store.load_session(session.id).await.expect("load session");

        assert_eq!(loaded.title, "Test session");
        assert_eq!(loaded.base_url.as_deref(), Some("http://127.0.0.1:11434"));
        assert_eq!(loaded.model.as_deref(), Some("updated-model"));
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].id, message.id);
        assert_eq!(loaded.events.len(), 1);
    }

    #[tokio::test]
    async fn jsonl_session_store_lists_recent_sessions_first() {
        let store = JsonlSessionStore::new(test_session_dir());
        let older = store
            .create_session(CreateSessionRequest {
                title: "Older".to_string(),
                workspace_root: None,
                base_url: None,
                model: None,
            })
            .await
            .expect("older session");
        let newer = store
            .create_session(CreateSessionRequest {
                title: "Newer".to_string(),
                workspace_root: None,
                base_url: None,
                model: None,
            })
            .await
            .expect("newer session");

        store
            .append_event(
                newer.id,
                SessionEvent::AgentTurnFailed {
                    run_id: Uuid::new_v4(),
                    message: "failed".to_string(),
                },
            )
            .await
            .expect("append event");

        let summaries = store.list_sessions().await.expect("list sessions");

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].id, newer.id);
        assert_eq!(summaries[1].id, older.id);
    }

    #[tokio::test]
    async fn jsonl_session_store_rejects_unknown_session_append() {
        let store = JsonlSessionStore::new(test_session_dir());
        let unknown_id = SessionId(Uuid::new_v4());

        let err = store
            .append_event(
                unknown_id,
                SessionEvent::AgentTurnCancelled {
                    run_id: Uuid::new_v4(),
                },
            )
            .await
            .expect_err("unknown session append should fail");
        let summaries = store.list_sessions().await.expect("list sessions");

        assert!(err.to_string().contains("open session log"));
        assert!(summaries.is_empty());
    }

    #[tokio::test]
    async fn jsonl_session_store_skips_unreadable_logs_when_listing() {
        let root = test_session_dir();
        let store = JsonlSessionStore::new(root.clone());
        let valid = store
            .create_session(CreateSessionRequest {
                title: "Valid".to_string(),
                workspace_root: None,
                base_url: None,
                model: None,
            })
            .await
            .expect("valid session");
        fs::create_dir_all(&root).expect("session root");
        fs::write(root.join("bad.jsonl"), "{not valid json}\n").expect("bad session log");

        let summaries = store.list_sessions().await.expect("list sessions");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, valid.id);
    }

    fn text_message(role: MessageRole, text: &str) -> ConversationMessage {
        ConversationMessage {
            id: Uuid::new_v4(),
            role,
            parts: vec![MessagePart::Text {
                text: text.to_string(),
            }],
        }
    }

    fn test_session_dir() -> PathBuf {
        std::env::temp_dir()
            .join("ollama-cowork-session-tests")
            .join(Uuid::new_v4().to_string())
    }
}
