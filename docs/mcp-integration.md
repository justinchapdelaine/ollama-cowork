# MCP Integration

Date: 2026-07-05

Ollama Cowork should be able to integrate Model Context Protocol (MCP) servers, but MCP must be a modular extension boundary, not a second permission system. MCP support should let users add context, prompts, and tools while preserving the app's existing workspace, policy, approval, sandbox, and audit controls.

## Product Goals

- Let users connect MCP servers without hard-coding them into the agent loop.
- Support server-provided prompts, resources, and tools through the same app-level registries used by built-in capabilities.
- Keep MCP server lifecycle, transport, trust state, permissions, and audit logs visible to the user.
- Let users enable MCP integrations per workspace by default, with global enablement as an explicit choice.
- Keep the implementation replaceable so future MCP protocol changes do not leak into model, sandbox, or UI code.

## Modular Architecture

MCP should sit behind replaceable interfaces:

- `ExtensionHost`: owns MCP client sessions and future non-MCP extension transports.
- `McpClient`: speaks the MCP protocol, negotiates versions/capabilities, and normalizes server data.
- `McpServerConfig`: describes one configured server's transport, command or URL, environment policy, trust state, and scope.
- `McpServerStore`: persists configured servers, transport settings, trust state, scopes, and enablement.
- `PromptRegistry`: exposes MCP prompts as user-selectable prompt templates or commands.
- `ResourceRegistry`: exposes MCP resources as selectable context, not implicit workspace access.
- `ToolRegistry`: exposes MCP tools only after schema validation, namespacing, and policy registration.
- `ToolPolicy`: classifies MCP tool calls before execution or forwarding.
- `ApprovalReviewer`: handles manual approval first, optional auto-review later.
- `SessionStore`: records server connections, capability negotiation, prompt/resource inclusion, tool calls, approvals, and errors.

Recommended flow:

```text
MCP Server -> ExtensionHost/McpClient -> PromptRegistry/ResourceRegistry/ToolRegistry -> ToolPolicy -> ApprovalReviewer -> WorkspaceRuntime
```

The acting model should not talk directly to an MCP server. It should see only normalized prompts, resources, and tools that Ollama Cowork has registered for the current session.

## Feature Mapping

- MCP prompts become user-controlled prompt templates, slash commands, or task starters.
- MCP resources become context entries users can search, inspect, include, remove, and audit.
- MCP tools become namespaced tool definitions with validated input schemas and optional output schemas.
- MCP roots map to the current workspace picker and any explicitly approved additional roots.
- MCP sampling maps to a separate host-controlled LLM request flow, with user review of prompts and returned results.
- MCP elicitation maps to explicit UI prompts. Form-mode elicitation must not be used for secrets, API keys, access tokens, payment credentials, or similarly sensitive data.
- MCP tasks and task-augmented execution should remain future work until the core approval and session model is mature.

## Safety Defaults

- MCP servers are disabled until the user adds and enables them.
- Workspace-scoped enablement is the default.
- Every server must show identity, transport, command or URL, version, declared capabilities, and requested host features before use.
- Server instructions, prompts, tool descriptions, annotations, icons, and resource metadata are untrusted unless they come from a trusted bundled source.
- Tool names must be namespaced per server, case-sensitive, stable, unique, and limited to predictable ASCII identifier characters.
- Tool inputs must be validated against declared schemas before calls are forwarded.
- Tool results should be validated against output schemas when provided.
- Tool calls must go through the same policy and approval path as built-in tools.
- Resource contents must not be sent to an MCP server, another MCP server, or the model without user-visible policy.
- Roots must never expose paths outside the selected workspace unless the user explicitly adds another root.
- Sampling requests must always be reviewable by the user. The user should be able to inspect and edit the prompt and review the generated response before it is returned to the requesting server.
- Elicitation requests must show which server is asking, what data will be sent, and clear decline/cancel options.
- HTTP-based MCP servers require explicit network policy. Local HTTP servers should prefer localhost bindings, and remote HTTP servers should require authentication when appropriate.
- stdio MCP servers should run under a controlled process runner with stdout/stderr handling, timeout, and cleanup.

## Transport Policy

MVP support should prefer stdio first because the app controls process launch and lifecycle. The stdio runner must:

- Send and receive only valid JSON-RPC messages over stdin/stdout.
- Complete MCP initialization, version negotiation, capability negotiation, and the `notifications/initialized` step before normal operations.
- Treat stderr as logs, not automatically as fatal errors.
- Capture server logs without mixing them into model context by default.
- Enforce timeout and process-tree cleanup.
- Store server command, arguments, working directory, environment policy, and trust state.
- Avoid passing secrets through ambient environment variables unless explicitly configured by the user.

Streamable HTTP support can come later. When it is added:

- Keep network access separate from sandbox command networking and Ollama model networking.
- Require allowlisted endpoints and clear server identity.
- Respect MCP protocol version headers.
- Reject or warn on local HTTP servers that bind beyond localhost.
- Require `Origin` validation for HTTP MCP servers to reduce DNS rebinding risk.
- Support authentication and token storage through a dedicated secret store rather than plain config.
- Treat localhost HTTP servers as privileged local surfaces, not automatically safe surfaces.

## MVP Shape

MCP does not need to be in the first sandbox MVP, but the architecture should reserve the extension boundary now.

A good staged plan is:

1. Define `ExtensionHost`, `McpClient`, `McpServerConfig`, and registry adapter traits.
2. Add config/storage for disabled MCP server definitions.
3. Implement server discovery and capability display without exposing tools to the model.
4. Add prompt and resource listing with explicit user selection.
5. Add read-only or low-risk tools through `ToolRegistry` and `ToolPolicy`.
6. Add approval-gated side-effecting tools after audit logs and session history are durable.
7. Add HTTP transport, authorization, sampling, elicitation, and tasks only after the stdio path is stable.

## Alignment With Current MCP Guidance

- MCP uses JSON-RPC and negotiates protocol version plus client/server capabilities during initialization.
- MCP standard transports are stdio and Streamable HTTP. Clients should support stdio where practical.
- MCP servers offer prompts, resources, and tools; clients may offer roots, sampling, and elicitation.
- MCP roots define filesystem boundaries and should map to the app's selected workspace roots.
- MCP prompts are intended to be user-controlled.
- MCP resources are application-driven context and should be selected or governed by host policy.
- MCP tools require caution, explicit consent, schema validation, and clear user understanding.
- MCP tool annotations and descriptions are untrusted unless they come from trusted servers.
- MCP sampling should keep a human in the loop with prompt review and response review.
- MCP elicitation requires clear server identity, user review, and decline/cancel options; form mode must not collect secrets.

## References

- MCP specification: https://modelcontextprotocol.io/specification/2025-11-25
- MCP lifecycle: https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle
- MCP transports: https://modelcontextprotocol.io/specification/2025-11-25/basic/transports
- MCP authorization: https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization
- MCP tools: https://modelcontextprotocol.io/specification/2025-11-25/server/tools
- MCP prompts: https://modelcontextprotocol.io/specification/2025-11-25/server/prompts
- MCP resources: https://modelcontextprotocol.io/specification/2025-11-25/server/resources
- MCP roots: https://modelcontextprotocol.io/specification/2025-11-25/client/roots
- MCP sampling: https://modelcontextprotocol.io/specification/2025-11-25/client/sampling
- MCP elicitation: https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation
