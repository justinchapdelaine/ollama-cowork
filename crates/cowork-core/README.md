# Spike core

Trusted, transport-independent Spike 001 policy and orchestration.

- `domain`: jobs, approvals, structured operations, and errors.
- `broker`: fail-closed `ToolBroker` application service.
- `ports`: `SandboxRunner` and `ArtifactPublisher` interfaces.

This crate imports no opencode, SRT, DOCX, HTTP, or Tauri types. Adapters translate those systems at the edge. `WorkflowController` owns framework-neutral job transitions through a replaceable `WorkflowJobFactory`, which supplies separate job-scoped `ModelSession`, `MutationAuthorization`, and `JobCleanup` handles, plus a replaceable `WorkflowEventSink`. Its internal phase machine permits completion only after approval, tool start, tool completion, and validated revised-artifact publication. Broker authorization is action-correlated, consumed before sandbox execution, and revoked if the subsequent model permission reply fails.
