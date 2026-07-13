# Broker transport

Transport-level request, revocation, and authentication helpers for the localhost broker boundary.

This crate knows the versioned broker request contract and owns the bounded localhost HTTP server lifecycle. Separate control authentication protects action-correlated approval decisions and unconsumed-approval revocation. It does not own jobs, approvals, SRT, DOCX operations, or Tauri integration. A development/proof host and the future desktop composition root can reuse it without importing one another.

`BrokerAuthorizationClient` implements the core `MutationAuthorization` port using only an explicit IPv4-loopback root origin without URL user information, redirects, or system proxies, plus the job-scoped control credential, bounded HTTP requests, and the versioned `/decision` and `/revoke` contracts. The credential is never exposed to opencode tools or the model-session adapter.
