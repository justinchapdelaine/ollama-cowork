# Engineering guidelines

Date: 2026-07-04

These guidelines capture the current implementation guardrails for the first Ollama Cowork build. They should be revisited when Tauri, Ollama, Rust, or the Windows runtime design changes materially.

## Source priorities

Use primary documentation before making framework or runtime decisions:

- Tauri docs for app structure, command IPC, capabilities, CSP, process model, and security boundaries.
- Ollama API docs for chat, thinking, tool calls, structured output, streaming, and model metadata.
- Rust and Cargo docs for package layout, error handling, and idiomatic library boundaries.
- Microsoft Learn for Windows process creation, tokens/users, ACLs, job objects, desktops, and firewall policy.

## Tauri and frontend boundary

- Keep privileged logic in Rust, not in the WebView.
- Expose a narrow Tauri command surface. Commands should validate typed inputs and return typed results.
- Treat the frontend as an untrusted caller across an IPC boundary.
- Use Tauri capabilities deliberately; do not grant broad plugin or shell permissions to the main window by default.
- Use a restrictive Content Security Policy and avoid loading remote UI code.
- Keep secrets, filesystem authority, model backend configuration, approval state, and sandbox control in the Rust core.
- Use frontend state for presentation only: message rendering, thinking disclosure, approval UI, diff display, settings forms, and progress views.

## Rust core design

- Build the core as a normal Rust library with small replaceable modules.
- Prefer traits for subsystem boundaries:
  - `ModelBackend`
  - `WorkspaceRuntime`
  - `ToolRegistry`
  - `ToolPolicy`
  - `ApprovalReviewer`
  - `DiffEngine`
  - `PatchApplier`
  - `SessionStore`
- Use explicit domain types instead of passing raw strings across major boundaries.
- Use `Result` for recoverable failures. Reserve panics for programmer bugs.
- Keep Windows-specific code behind runtime modules so future macOS, WSL, or VM-backed runtimes do not leak into orchestration code.
- Avoid `unsafe` unless a Windows API call truly requires it; isolate and document any `unsafe` block.
- Add focused tests for policy decisions, Ollama response parsing, path validation, diff generation, and patch application before broad UI tests.

## Ollama backend

- Support configurable Ollama base URLs, including `http://127.0.0.1:11434` and LAN-hosted servers.
- Treat host-side Ollama access separately from sandbox command networking.
- Parse `message.thinking`, `message.content`, and `message.tool_calls` into separate app message types.
- Keep thinking enabled by default and render it as a collapsible UI block.
- Execute only validated structured tool calls. Never execute text from `message.content` or `message.thinking`.
- Preserve model timing and token metadata in session logs for latency/debugging.
- Handle streaming later, but keep the first backend implementation compatible with both streaming and non-streaming responses.

## Tool and approval policy

- The acting model can request tools, but policy decides whether they run.
- Manual approval is the MVP default for network, install, destructive, or host-affecting actions.
- Auto-review must be a separate reviewer layer, not self-approval by the acting model.
- Log every approval request and decision with command, cwd, requested capability, reviewer, reason, and result.
- Keep localhost model access, LAN model access, and sandbox command internet access as separate policy categories.

## Windows runtime

- Start with elevated OS-level sandboxing using dedicated low-privilege users/groups.
- Keep copy-then-patch as the default workspace model.
- Run side-effecting commands only inside copied workspaces.
- Use ACL boundaries so the sandbox identity can access the copied workspace and required read roots only.
- Block network by default for sandbox command users.
- Capture stdout, stderr, exit code, duration, and timeout state for every command.
- Use Windows job objects or equivalent process-tree management for timeout cleanup.
- Do not create users, alter firewall rules, or change system ACLs without explicit setup/approval.
- Treat OS-level sandboxing as practical protection for local coding tasks, not malware-grade containment.

## Diff and apply

- MVP approval granularity is whole-patch approval.
- Generate a structured change inventory in addition to raw patch text.
- Preserve room for future per-file and per-hunk approval.
- Apply changes from the copied workspace back to the source folder only after approval.
- Keep patch application deterministic and auditable.

## References

- Tauri start and project structure: https://tauri.app/start/
- Tauri security overview: https://tauri.app/security/
- Tauri capabilities: https://tauri.app/security/capabilities/
- Tauri command IPC: https://tauri.app/develop/calling-rust/
- Tauri process model: https://tauri.app/concept/process-model/
- Ollama API: https://docs.ollama.com/api
- Ollama API source mirror: https://github.com/ollama/ollama/blob/main/docs/api.md
- Rust API Guidelines: https://rust-lang.github.io/api-guidelines/
- Rust error handling: https://doc.rust-lang.org/book/ch09-00-error-handling.html
- Cargo package layout: https://doc.rust-lang.org/cargo/guide/project-layout.html
- Microsoft CreateProcessWithLogonW: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createprocesswithlogonw
- Microsoft CreateRestrictedToken: https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-createrestrictedtoken
- Microsoft Job Objects: https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
- Microsoft Windows Firewall: https://learn.microsoft.com/en-us/windows/security/operating-system-security/network-security/windows-firewall/
