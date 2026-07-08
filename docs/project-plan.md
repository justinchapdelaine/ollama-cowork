# Project Plan

This is the living handoff document for Ollama Cowork. Keep it continuously updated as decisions, test results, priorities, and implementation status change. If it starts drifting from the code or conversation, update this file before relying on it for planning.

Last updated: 2026-07-08

## Purpose

Ollama Cowork is a local-first desktop coworking agent that uses Ollama models, starts with read-only workspace inspection, and is being built toward sandboxed, reviewable code changes.

This document should answer "where are we, what did we decide, and what should happen next?" without needing the original chat history.

## Maintenance And Privacy

- Update this document whenever goals, priorities, decisions, test results, or implementation status materially change.
- Keep this document suitable for normal repo commits.
- Do not include private IP addresses, personal filesystem paths, usernames, emails, access tokens, machine-specific account names, local sandbox SIDs, or other sensitive host details.
- Put sensitive local setup notes in gitignored local docs instead, and keep only sanitized conclusions here.
- If this document conflicts with the code, tests, or deeper planning docs, resolve the conflict before using it as a handoff.

## MVP Goals

- Provide a usable Tauri desktop chat surface for a local or LAN Ollama model.
- Let users choose an explicit workspace root before tools can operate.
- Keep model calls in the host app, separate from sandbox command networking.
- Start with read-only workspace tools.
- Add streaming, durable sessions, approvals, and copy-then-patch editing in modular layers.
- Use an elevated Windows OS-level sandbox for side-effecting work once the runtime layer is ready.
- Keep skills, MCP integrations, model backends, approval reviewers, and runtime strategies replaceable behind clear interfaces.

## Settled Decisions

- Desktop shell: Tauri v2.
- Backend/core: Rust.
- Frontend: TypeScript with Vite.
- Initial serious model baseline: `gemma4:12b` through Ollama.
- Thinking should be enabled by default and shown in collapsible UI.
- Workspace selection is explicit through a native folder picker.
- Tool paths are workspace-relative.
- Read-only tools are allowed first: `list_files`, `read_file`, and `search_files`.
- Side-effecting work should use copy-then-patch before touching the selected workspace.
- Manual approvals are the MVP default.
- Auto-review can be added later only as a separate reviewer layer; the acting model must not approve its own risky actions.
- Network, install, destructive, and host-affecting commands require approval.
- Skills and MCP servers must not bypass `ToolRegistry`, `ToolPolicy`, approvals, sandboxing, or audit logs.

## Current Implementation

- Latest committed checkpoint: `aff8473` on `branch/init`, `feat(sessions): add recent session reloads`.
- Tauri app launches with workspace picker, Ollama settings, diagnostics, and chat UI.
- Ollama backend can probe `/api/version`, list `/api/tags`, and run both non-streaming and streaming `/api/chat` turns.
- Agent loop supports thinking, tool calls, tool results, final assistant messages, bounded tool iterations, run-event streaming, and non-streaming fallback.
- Run cancellation is tracked through a Rust `AgentRunStore`.
- Durable session storage has an initial modular `SessionStore` boundary with a JSONL-backed local implementation for session metadata and completed/failed/cancelled agent-turn events.
- Context compaction summarizes older history while preserving recent messages.
- Read-only local tools support bounded file reads, file search, hidden/generated entry reporting, and cancellation checks.
- UI renders conversation history, streaming thinking/content deltas, collapsible thinking, tool call/result blocks, status output, recent sessions, and responsive/narrow-window layouts; it creates a new durable session for each selected workspace, persists completed turns, and can reload saved sessions.
- Approval/runtime scaffold work has started with typed capability categories, approval request/decision message shapes, default policy behavior, a policy-enforced tool boundary around read-only tools, pending approval storage, and a basic manual approval UI surface.
- Planning docs exist for sandboxing, engineering guidelines, skills/extensions, and MCP integration.

## Recent Test Evidence

- `gemma4:12b` on the LAN Ollama server successfully produced structured tool calls and thinking.
- Computer Use smoke test passed against the real Tauri desktop window:
  - selected the repo workspace through the native folder picker;
  - ran `Run Tool Probe`;
  - sent a real chat prompt;
  - verified thinking, tool call, tool result, final answer, status counters, and run-control states;
  - resized to the narrow breakpoint and verified diagnostics remain reachable.
- Streaming smoke test passed against the real Tauri desktop window:
  - selected the repo workspace through the native folder picker;
  - sent a prompt through the streamed agent command;
  - verified immediate user/assistant event rendering, streamed thinking/tool call display, tool result rendering, final assistant content, and completion counters.
- Session reload smoke test passed against the real Tauri desktop window:
  - listed existing JSONL-backed recent sessions;
  - loaded a saved session and restored its workspace, conversation, model, and base URL;
  - sent a new streamed prompt from the loaded session;
  - verified recent session controls were disabled during the active run;
  - verified the saved session updated in the recent list and reloaded with the last-used runtime settings.
- Verification commands passed after the latest agent-loop work:
  - `npm.cmd test`
  - `cargo fmt -- --check`
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
  - `npm.cmd run build`

## Next Milestones

1. Build the approval/runtime scaffold.
   - Define capability categories for reads, writes, commands, network, installs, destructive operations, and host-affecting actions.
   - Add approval request/result message types in the session model.
   - Keep side-effecting capabilities disabled by default.
   - Keep reviewer, policy, session logging, and runtime execution boundaries modular.
   - Next: generate approval requests from side-effecting tools/runtime commands and resume or deny those actions based on the stored decision.

2. Implement copy-then-patch.
   - Create a copied workspace runtime.
   - Generate reviewable diffs.
   - Apply approved whole patches back to the source workspace.
   - Preserve room for future per-file and per-hunk approvals.

3. Prototype the elevated Windows OS sandbox runner.
   - Start without system mutations where possible.
   - Later, with explicit admin approval, create dedicated sandbox users/groups, ACLs, offline-by-default network controls, cleanup, and launch behavior.

4. Continue durable session/audit improvements.
   - Extend the JSONL session store toward full run-event/audit capture.
   - Persist approvals, command decisions, tool calls, and summaries.
   - Keep improving conversation reload with search, pruning, and clearer session metadata.
   - Keep UI state separate from session history.

5. Continue streaming/UI polish.
   - Keep non-streaming as a fallback while the streamed path matures.
   - Continue routing frontend updates through modular run-event handling.
   - Add more regression coverage for frontend event reduction and cancellation UX.

## Backlog

- Improve context compaction with a structured `SessionSummary`.
- Add streaming progress and cancellation UX polish.
- Add richer session list/sidebar controls such as search, rename, delete, and pinning.
- Add a model picker backed by `/api/tags`.
- Add read-only repository awareness such as Git status, file tree search, and safe source previews.
- Add write/edit capability only through approval-gated patch proposals.
- Add audit logs for tool calls, approvals, command outputs, and applied patches.
- Add workspace-scoped skills and prompt templates.
- Add disabled-by-default MCP server definitions behind `ExtensionHost`.
- Add stdio MCP experiments after session/audit basics exist.
- Add better responsive UI testing around narrow windows and long tool output.

## Open Questions

- What exact local storage format should sessions use first: SQLite, file-backed JSONL, or a hybrid?
- How much of the runtime/event model should be implemented before streaming?
- What is the first safe write/edit scenario for copy-then-patch testing?
- How should the app present hidden/generated entries to the model and user long term?
- Which Windows sandbox setup steps can be automated safely, and which require explicit admin setup?
- What should the initial user-facing skill format look like?

## Reference Docs

- [Windows sandbox spike](windows-sandbox-spike.md)
- [Engineering guidelines](engineering-guidelines.md)
- [Skills and extensions](skills-and-extensions.md)
- [MCP integration](mcp-integration.md)
