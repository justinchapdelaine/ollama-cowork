use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("model backend error: {0}")]
    ModelBackend(String),

    #[error("runtime error: {0}")]
    Runtime(String),

    #[error("tool policy denied action: {0}")]
    PolicyDenied(String),

    #[error("approval required: {0}")]
    ApprovalRequired(String),

    #[error("diff/apply error: {0}")]
    DiffApply(String),

    #[error("session store error: {0}")]
    SessionStore(String),

    #[error("agent run cancelled")]
    Cancelled,
}
