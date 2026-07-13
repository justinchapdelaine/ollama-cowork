# Broker transport

Transport-level request, revocation, and authentication helpers for the localhost broker boundary.

This crate knows the versioned broker request contract and provides the authenticated IPv4-loopback server. Separate control authentication protects action-correlated approval decisions and unconsumed-approval revocation. It does not own jobs, approvals, SRT, DOCX operations, process lifetime, or Tauri integration.

The server entry point is intentionally blocking. Production composition runs the
narrow one-job broker host under `process-supervisor`, so a stalled request or
tool execution cannot make desktop shutdown wait on an uninterruptible in-process
thread. Closing the supervisor's Windows Job Object terminates the complete host
process tree. Server code is behind the opt-in `server` Cargo feature, which only
the broker-host executable enables; the desktop dependency exposes the client
contract but cannot accidentally embed the blocking server.

`BrokerAuthorizationClient` implements the core `MutationAuthorization` port using only an explicit IPv4-loopback root origin without URL user information, redirects, or system proxies, plus the job-scoped control credential, bounded HTTP requests, and the versioned `/decision` and `/revoke` contracts. The credential is never exposed to opencode tools or the model-session adapter.
