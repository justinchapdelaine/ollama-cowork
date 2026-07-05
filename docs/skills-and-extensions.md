# Skills and Extensions

Date: 2026-07-05

Ollama Cowork should support user-manageable skills and extensions, but they must remain modular and must not become a shortcut around tool policy, approvals, sandboxing, or Tauri capabilities.

## Product Goals

- Let users add, remove, enable, disable, and inspect skills.
- Support workspace-scoped skills as the default, with global skills as an explicit user choice.
- Keep built-in skills and user-installed skills behind the same registry interfaces.
- Preserve future room for local folders, imported bundles, marketplace-style installs, and MCP-style external servers.
- Make skills explainable in the UI: users should be able to see what a skill adds before enabling it.
- Track skill identity, source, version, update state, and trust scope separately from the skill's human-readable name.

## Capability Model

Skills can provide:

- Instructions and system prompt fragments.
- Reusable workflows and prompt templates.
- Tool definitions that map to host-implemented tools.
- Resource descriptors, examples, schemas, and documentation.
- UI affordances such as slash commands or task starters.

Skills must not directly grant:

- Filesystem access.
- Shell or process execution.
- Network access.
- Tauri plugin permissions.
- Sandbox escape or host mutation rights.
- Approval bypasses.
- Access to workspace roots beyond the active workspace selection.
- LLM sampling, user elicitation, or external-server communication without explicit host policy and user consent.

Executable behavior still flows through the normal app layers:

```text
SkillRegistry -> ToolRegistry -> ToolPolicy -> ApprovalReviewer -> WorkspaceRuntime
```

This keeps skills useful without treating natural-language skill instructions as a security boundary.

## Modular Interfaces

The first architecture should leave space for these replaceable pieces:

- `SkillRegistry`: discovers enabled built-in and user-installed skills.
- `SkillStore`: persists skill manifests, source paths, trust state, and enablement scope.
- `SkillManifest`: declares prompts, tools, resources, required capabilities, and version metadata.
- `SkillLoader`: loads skill instructions/resources with validation and size limits.
- `SkillResolver`: handles identity, version constraints, duplicates, conflicts, and update provenance.
- `ExtensionHost`: future boundary for external extension servers or MCP-style integrations.

These should be separate from `ToolRegistry`. A skill may request that a tool be exposed, but the app decides whether the tool exists, what policy applies, and whether approval is required.

## Safety Defaults

- Skills are disabled until the user installs or enables them.
- User-installed skills should be scoped to a workspace by default.
- Skills must declare requested capabilities in a manifest.
- Capability requests should be shown before enablement and re-confirmed if they change.
- Skill instructions, descriptions, and metadata are untrusted input.
- The app should validate manifests structurally and never execute scripts while merely discovering a skill.
- Tool declarations from skills should include input schemas, and tool results should be validated against output schemas when one is declared.
- Skill-contributed tool names should be stable, unique within their namespace, case-sensitive, and limited to predictable ASCII identifier characters.
- Tool annotations, descriptions, icons, and display metadata should be treated as advisory and untrusted unless they come from a trusted bundled skill.
- Tool calls from skills are audited exactly like built-in tool calls.
- Prompt/resource context from a skill should be inspectable and removable from a session.
- Resource inclusion should default to explicit user or app selection, with automatic inclusion treated as policy-governed context sharing.
- Any future MCP-style Roots, Sampling, or Elicitation support must be exposed as separate policy-controlled host features, not implicit skill permissions.
- Conflicting skill names, duplicate tool names, or lookalike capabilities should require explicit resolution.

## MVP Shape

The MVP does not need a marketplace or scriptable extension runtime. A good first step is:

1. Define `SkillManifest` and `SkillRegistry` domain types.
2. Add a built-in skill registry for bundled workflows.
3. Add a workspace-local skill folder convention later, such as `.ollama-cowork/skills/`.
4. Show enabled skills in the session settings.
5. Let skills contribute prompt templates before allowing them to contribute tool definitions.
6. Add install/update/remove flows only after manifest validation, conflict resolution, and trust-state UI exist.

This staged approach gives users customization early while keeping executable power behind the existing Rust-controlled policy layers.

## Alignment With Current Guidance

- Tauri capabilities should stay narrow and window-scoped; a skill cannot expand frontend permissions by itself.
- MCP separates prompts, resources, and tools; this maps well to skill-provided workflows, context, and executable functions.
- MCP also separates client-side roots, sampling, and elicitation; these should map to explicit Ollama Cowork host policies rather than skill-controlled behavior.
- MCP trust guidance treats tools and data access as requiring user control, clear UI, input validation, access control, and audit logging.
- MCP tool schemas support structured input and output validation; skill-contributed tool contracts should follow that pattern.
- Prompt/skill metadata can influence agent behavior, so discovery and enablement must treat natural-language descriptions as untrusted.
- Dedicated MCP server integration policy lives in [MCP integration](mcp-integration.md); skills may reference MCP-style concepts, but MCP server lifecycle belongs behind `ExtensionHost`.

## References

- Tauri capabilities: https://tauri.app/security/capabilities/
- MCP specification: https://modelcontextprotocol.io/specification/2025-11-25
- MCP tools: https://modelcontextprotocol.io/specification/2025-11-25/server/tools
- MCP prompts: https://modelcontextprotocol.io/specification/2025-11-25/server/prompts
- MCP resources: https://modelcontextprotocol.io/specification/2025-11-25/server/resources
- MCP roots: https://modelcontextprotocol.io/specification/2025-11-25/client/roots
- MCP sampling: https://modelcontextprotocol.io/specification/2025-11-25/client/sampling
- MCP elicitation: https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation
