# Spike core

Trusted, transport-independent Spike 001 policy and orchestration.

- `domain`: jobs, approvals, structured operations, and errors.
- `broker`: fail-closed `ToolBroker` application service.
- `ports`: `SandboxRunner` and `ArtifactPublisher` interfaces.

This crate imports no opencode, SRT, DOCX, HTTP, or Tauri types. Adapters translate those systems at the edge. A one-time approval is consumed before sandbox execution begins, preventing retries or concurrent requests from reusing it.
