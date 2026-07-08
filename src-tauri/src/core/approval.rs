use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

use crate::core::error::{AppError, AppResult};
use crate::core::session::{SessionEvent, SessionId, SessionStore};

#[async_trait]
pub trait ApprovalReviewer: Send + Sync {
    async fn review(&self, request: ApprovalRequest) -> AppResult<ApprovalDecision>;
}

pub trait ApprovalPolicy: Send + Sync {
    fn evaluate(&self, request: &ApprovalRequest) -> ApprovalRequirement;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub summary: String,
    pub subject: ApprovalSubject,
    pub requested_capabilities: Vec<RequestedCapability>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedCapability {
    WorkspaceRead,
    WorkspaceWrite,
    Command,
    NetworkAccess,
    Install,
    DestructiveFilesystem,
    HostMutation,
    SandboxEscape,
}

impl RequestedCapability {
    pub fn is_auto_allowed_by_default(&self) -> bool {
        matches!(self, Self::WorkspaceRead)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorkspaceRead => "workspace_read",
            Self::WorkspaceWrite => "workspace_write",
            Self::Command => "command",
            Self::NetworkAccess => "network_access",
            Self::Install => "install",
            Self::DestructiveFilesystem => "destructive_filesystem",
            Self::HostMutation => "host_mutation",
            Self::SandboxEscape => "sandbox_escape",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApprovalSubject {
    ToolCall {
        name: String,
        arguments: Value,
    },
    RuntimeCommand {
        program: String,
        args: Vec<String>,
        cwd: String,
    },
    PatchApply {
        summary: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequirement {
    Allow,
    RequireManualApproval,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecision {
    pub id: Uuid,
    pub request_id: Uuid,
    pub approved: bool,
    pub reviewer: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub request: ApprovalRequest,
    pub session_id: Option<crate::core::session::SessionId>,
    pub run_id: Option<Uuid>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedApproval {
    pub request: ApprovalRequest,
    pub session_id: Option<crate::core::session::SessionId>,
    pub run_id: Option<Uuid>,
    pub decision: ApprovalDecision,
    pub created_at_ms: u64,
    pub resolved_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApprovalSubmission {
    Allowed { request: ApprovalRequest },
    PendingManualApproval { pending: PendingApproval },
}

#[derive(Debug, Default)]
pub struct ApprovalRequestStore {
    pending: Mutex<HashMap<Uuid, PendingApproval>>,
}

impl ApprovalRequestStore {
    pub fn request(
        &self,
        request: ApprovalRequest,
        session_id: Option<crate::core::session::SessionId>,
        run_id: Option<Uuid>,
    ) -> AppResult<PendingApproval> {
        let pending = PendingApproval {
            request,
            session_id,
            run_id,
            created_at_ms: now_ms(),
        };
        let mut pending_requests = self
            .pending
            .lock()
            .map_err(|err| AppError::Runtime(format!("approval store poisoned: {err}")))?;
        if pending_requests.contains_key(&pending.request.id) {
            return Err(AppError::Runtime(format!(
                "approval request already pending: {}",
                pending.request.id
            )));
        }
        pending_requests.insert(pending.request.id, pending.clone());
        Ok(pending)
    }

    pub fn list(&self) -> AppResult<Vec<PendingApproval>> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|err| AppError::Runtime(format!("approval store poisoned: {err}")))?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        pending.sort_by_key(|approval| approval.created_at_ms);
        Ok(pending)
    }

    pub fn prepare_resolution(
        &self,
        request_id: Uuid,
        approved: bool,
        reviewer: impl Into<String>,
        reason: impl Into<String>,
    ) -> AppResult<ResolvedApproval> {
        let pending = self
            .pending
            .lock()
            .map_err(|err| AppError::Runtime(format!("approval store poisoned: {err}")))?
            .get(&request_id)
            .cloned()
            .ok_or_else(|| AppError::Runtime(format!("unknown approval request: {request_id}")))?;
        let decision = ApprovalDecision {
            id: Uuid::new_v4(),
            request_id,
            approved,
            reviewer: reviewer.into(),
            reason: reason.into(),
        };

        Ok(ResolvedApproval {
            request: pending.request,
            session_id: pending.session_id,
            run_id: pending.run_id,
            decision,
            created_at_ms: pending.created_at_ms,
            resolved_at_ms: now_ms(),
        })
    }

    pub fn complete(&self, request_id: Uuid) -> AppResult<()> {
        self.pending
            .lock()
            .map_err(|err| AppError::Runtime(format!("approval store poisoned: {err}")))?
            .remove(&request_id)
            .map(|_| ())
            .ok_or_else(|| AppError::Runtime(format!("unknown approval request: {request_id}")))
    }
}

#[derive(Debug, Clone, Default)]
pub struct DefaultApprovalPolicy;

impl ApprovalPolicy for DefaultApprovalPolicy {
    fn evaluate(&self, request: &ApprovalRequest) -> ApprovalRequirement {
        if request.requested_capabilities.is_empty() {
            return ApprovalRequirement::Deny;
        }

        if request
            .requested_capabilities
            .iter()
            .all(RequestedCapability::is_auto_allowed_by_default)
        {
            return ApprovalRequirement::Allow;
        }

        ApprovalRequirement::RequireManualApproval
    }
}

pub async fn submit_approval_request<P, S>(
    request: ApprovalRequest,
    session_id: Option<SessionId>,
    run_id: Option<Uuid>,
    policy: &P,
    approvals: &ApprovalRequestStore,
    sessions: Option<&S>,
) -> AppResult<ApprovalSubmission>
where
    P: ApprovalPolicy,
    S: SessionStore + ?Sized,
{
    match policy.evaluate(&request) {
        ApprovalRequirement::Allow => Ok(ApprovalSubmission::Allowed { request }),
        ApprovalRequirement::RequireManualApproval => {
            let pending = approvals.request(request.clone(), session_id, run_id)?;
            if let (Some(session_id), Some(sessions)) = (session_id, sessions) {
                if let Err(err) = sessions
                    .append_event(
                        session_id,
                        SessionEvent::ApprovalRequested { run_id, request },
                    )
                    .await
                {
                    let _ = approvals.complete(pending.request.id);
                    return Err(err);
                }
            }

            Ok(ApprovalSubmission::PendingManualApproval { pending })
        }
        ApprovalRequirement::Deny => Err(AppError::PolicyDenied(format!(
            "{}: {}",
            request.summary, request.reason
        ))),
    }
}

#[derive(Debug, Clone, Default)]
pub struct DeferredApprovalReviewer;

#[async_trait]
impl ApprovalReviewer for DeferredApprovalReviewer {
    async fn review(&self, request: ApprovalRequest) -> AppResult<ApprovalDecision> {
        Err(AppError::ApprovalRequired(format!(
            "{} ({})",
            request.summary, request.reason
        )))
    }
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
    use crate::core::session::{CreateSessionRequest, JsonlSessionStore};
    use std::path::PathBuf;

    #[test]
    fn default_policy_allows_workspace_reads() {
        let request = tool_request(
            "read_file",
            vec![RequestedCapability::WorkspaceRead],
            "read a selected workspace file",
        );

        assert_eq!(
            DefaultApprovalPolicy.evaluate(&request),
            ApprovalRequirement::Allow
        );
    }

    #[test]
    fn default_policy_requires_manual_approval_for_side_effects() {
        let request = tool_request(
            "apply_patch",
            vec![
                RequestedCapability::WorkspaceRead,
                RequestedCapability::WorkspaceWrite,
            ],
            "modify a selected workspace file",
        );

        assert_eq!(
            DefaultApprovalPolicy.evaluate(&request),
            ApprovalRequirement::RequireManualApproval
        );
    }

    #[test]
    fn default_policy_denies_unclassified_actions() {
        let request = tool_request("unknown", Vec::new(), "unclassified action");

        assert_eq!(
            DefaultApprovalPolicy.evaluate(&request),
            ApprovalRequirement::Deny
        );
    }

    #[test]
    fn approval_store_lists_and_resolves_pending_requests() {
        let store = ApprovalRequestStore::default();
        let request = tool_request(
            "apply_patch",
            vec![RequestedCapability::WorkspaceWrite],
            "apply proposed patch",
        );
        let request_id = request.id;

        store
            .request(request.clone(), None, None)
            .expect("store approval request");
        let pending = store.list().expect("list pending approvals");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request.id, request_id);

        let resolved = store
            .prepare_resolution(request_id, true, "user", "looks good")
            .expect("resolve approval");

        assert_eq!(resolved.request.id, request_id);
        assert_eq!(resolved.decision.request_id, request_id);
        assert!(resolved.decision.approved);
        assert_eq!(store.list().expect("list before complete").len(), 1);
        store.complete(request_id).expect("complete approval");
        assert!(store.list().expect("list after resolve").is_empty());
    }

    #[test]
    fn approval_store_rejects_unknown_resolutions() {
        let store = ApprovalRequestStore::default();
        let err = store
            .prepare_resolution(Uuid::new_v4(), false, "user", "not found")
            .expect_err("unknown approval should fail");

        assert!(err.to_string().contains("unknown approval request"));
    }

    #[test]
    fn approval_store_rejects_duplicate_pending_request_ids() {
        let store = ApprovalRequestStore::default();
        let request = tool_request(
            "apply_patch",
            vec![RequestedCapability::WorkspaceWrite],
            "apply proposed patch",
        );

        store
            .request(request.clone(), None, None)
            .expect("store first approval request");
        let err = store
            .request(request, None, None)
            .expect_err("duplicate request ids should fail");

        assert!(err.to_string().contains("already pending"));
        assert_eq!(store.list().expect("list pending").len(), 1);
    }

    #[tokio::test]
    async fn submit_approval_request_allows_auto_allowed_requests() {
        let approvals = ApprovalRequestStore::default();
        let request = tool_request(
            "read_file",
            vec![RequestedCapability::WorkspaceRead],
            "read a selected workspace file",
        );
        let request_id = request.id;

        let submitted = submit_approval_request(
            request,
            None,
            None,
            &DefaultApprovalPolicy,
            &approvals,
            Option::<&JsonlSessionStore>::None,
        )
        .await
        .expect("submit auto-allowed request");

        assert!(matches!(
            submitted,
            ApprovalSubmission::Allowed { request } if request.id == request_id
        ));
        assert!(approvals.list().expect("list pending").is_empty());
    }

    #[tokio::test]
    async fn submit_approval_request_stores_and_logs_manual_requests() {
        let approvals = ApprovalRequestStore::default();
        let sessions = JsonlSessionStore::new(test_session_dir());
        let session = sessions
            .create_session(CreateSessionRequest {
                title: "Approval gate".to_string(),
                workspace_root: None,
                base_url: None,
                model: None,
            })
            .await
            .expect("create session");
        let request = tool_request(
            "run_command",
            vec![RequestedCapability::Command],
            "runtime command execution requires approval",
        );
        let request_id = request.id;
        let run_id = Uuid::new_v4();

        let submitted = submit_approval_request(
            request,
            Some(session.id),
            Some(run_id),
            &DefaultApprovalPolicy,
            &approvals,
            Some(&sessions),
        )
        .await
        .expect("submit manual request");

        assert!(matches!(
            submitted,
            ApprovalSubmission::PendingManualApproval { pending }
                if pending.request.id == request_id && pending.run_id == Some(run_id)
        ));
        assert_eq!(approvals.list().expect("list pending").len(), 1);

        let loaded = sessions
            .load_session(session.id)
            .await
            .expect("load session");
        assert_eq!(loaded.events.len(), 1);
        assert!(matches!(
            &loaded.events[0].event,
            SessionEvent::ApprovalRequested { request, .. } if request.id == request_id
        ));
    }

    #[tokio::test]
    async fn submit_approval_request_rolls_back_pending_when_session_log_fails() {
        let approvals = ApprovalRequestStore::default();
        let sessions = JsonlSessionStore::new(test_session_dir());
        let request = tool_request(
            "run_command",
            vec![RequestedCapability::Command],
            "runtime command execution requires approval",
        );

        let err = submit_approval_request(
            request,
            Some(SessionId(Uuid::new_v4())),
            None,
            &DefaultApprovalPolicy,
            &approvals,
            Some(&sessions),
        )
        .await
        .expect_err("unknown session should fail");

        assert!(err.to_string().contains("session store error"));
        assert!(approvals.list().expect("list pending").is_empty());
    }

    fn tool_request(
        name: &str,
        requested_capabilities: Vec<RequestedCapability>,
        reason: &str,
    ) -> ApprovalRequest {
        ApprovalRequest {
            id: Uuid::new_v4(),
            summary: format!("Run tool `{name}`"),
            subject: ApprovalSubject::ToolCall {
                name: name.to_string(),
                arguments: serde_json::json!({}),
            },
            requested_capabilities,
            reason: reason.to_string(),
        }
    }

    fn test_session_dir() -> PathBuf {
        std::env::temp_dir()
            .join("ollama-cowork-approval-tests")
            .join(Uuid::new_v4().to_string())
    }
}
