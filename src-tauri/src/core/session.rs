use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::AppResult;
use crate::core::messages::ConversationMessage;

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn create_session(&self, title: String) -> AppResult<SessionId>;
    async fn append_message(
        &self,
        session_id: SessionId,
        message: ConversationMessage,
    ) -> AppResult<()>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SessionId(pub Uuid);
